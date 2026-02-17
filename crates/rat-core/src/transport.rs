use std::cmp;
use std::io;
use std::time::Duration;

use bytes::BytesMut;
use futures_util::StreamExt;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::codec::{Decoder, FramedRead};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

#[derive(Clone, Debug)]
pub struct ListenerOptions {
    pub reconnect: Duration,
    pub reconnect_max: Duration,
    pub dial_timeout: Duration,
    pub strip_jlink_banner: bool,
    pub reader_buf_bytes: usize,
}

impl Default for ListenerOptions {
    fn default() -> Self {
        Self {
            reconnect: Duration::from_secs(1),
            reconnect_max: Duration::from_secs(30),
            dial_timeout: Duration::from_secs(5),
            strip_jlink_banner: false,
            reader_buf_bytes: 65_536,
        }
    }
}

#[derive(Default)]
struct ZeroDelimitedFrameCodec;

impl Decoder for ZeroDelimitedFrameCodec {
    type Item = Vec<u8>;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let Some(delimiter_index) = src.iter().position(|byte| *byte == 0) else {
            return Ok(None);
        };

        let mut frame = src.split_to(delimiter_index + 1);
        frame.truncate(delimiter_index);
        Ok(Some(frame.to_vec()))
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.is_empty() {
            return Ok(None);
        }
        src.clear();
        Ok(None)
    }
}

pub fn spawn_listener(
    shutdown: CancellationToken,
    addr: String,
    out: mpsc::Sender<Vec<u8>>,
    options: ListenerOptions,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut attempts: u32 = 0;
        let reader_buf_bytes = normalize_reader_buf_bytes(options.reader_buf_bytes);

        while !shutdown.is_cancelled() {
            let stream =
                tokio::time::timeout(options.dial_timeout, TcpStream::connect(&addr)).await;
            let stream = match stream {
                Ok(Ok(stream)) => stream,
                Ok(Err(err)) => {
                    attempts = attempts.saturating_add(1);
                    warn!(%addr, attempt = attempts, error = %err, "transport connect failed");
                    wait_backoff(&shutdown, &options, attempts).await;
                    continue;
                }
                Err(_) => {
                    attempts = attempts.saturating_add(1);
                    warn!(%addr, attempt = attempts, timeout_ms = options.dial_timeout.as_millis(), "transport connect timeout");
                    wait_backoff(&shutdown, &options, attempts).await;
                    continue;
                }
            };

            attempts = 0;
            info!(%addr, "transport connected");

            match handle_connection(
                stream,
                &out,
                shutdown.clone(),
                options.strip_jlink_banner,
                reader_buf_bytes,
            )
            .await
            {
                Ok(()) => {
                    attempts = attempts.saturating_add(1);
                    info!(%addr, attempt = attempts, "transport connection closed");
                }
                Err(err) => {
                    attempts = attempts.saturating_add(1);
                    warn!(%addr, attempt = attempts, error = %err, "transport connection error");
                }
            }

            wait_backoff(&shutdown, &options, attempts).await;
        }
    })
}

async fn handle_connection(
    mut stream: TcpStream,
    out: &mpsc::Sender<Vec<u8>>,
    shutdown: CancellationToken,
    strip_jlink_banner: bool,
    reader_buf_bytes: usize,
) -> Result<(), io::Error> {
    if strip_jlink_banner {
        strip_jlink_banner_line(&mut stream).await?;
    }

    let mut framed =
        FramedRead::with_capacity(stream, ZeroDelimitedFrameCodec::default(), reader_buf_bytes);

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                return Ok(());
            }
            maybe_frame = framed.next() => {
                let Some(frame_result) = maybe_frame else {
                    return Ok(());
                };

                match frame_result {
                    Ok(frame) => {
                        if frame.is_empty() {
                            continue;
                        }

                        if out.send(frame).await.is_err() {
                            debug!("frame consumer dropped");
                            return Ok(());
                        }
                    }
                    Err(err) => {
                        return Err(err);
                    }
                }
            }
        }
    }
}

async fn strip_jlink_banner_line(stream: &mut TcpStream) -> Result<(), io::Error> {
    let mut probe = [0_u8; 128];
    let probed =
        match tokio::time::timeout(Duration::from_millis(200), stream.peek(&mut probe)).await {
            Ok(Ok(count)) => count,
            Ok(Err(err)) => return Err(err),
            Err(_) => return Ok(()),
        };

    if probed == 0 || !looks_like_jlink_banner(&probe[..probed]) {
        return Ok(());
    }

    let mut consumed = 0_usize;
    let mut byte = [0_u8; 1];
    while consumed < 1024 {
        let read = tokio::time::timeout(Duration::from_secs(1), stream.read(&mut byte)).await;
        let read = match read {
            Ok(Ok(size)) => size,
            Ok(Err(err)) => return Err(err),
            Err(_) => break,
        };

        if read == 0 {
            break;
        }
        consumed += read;
        if byte[0] == b'\n' {
            break;
        }
    }

    Ok(())
}

fn looks_like_jlink_banner(bytes: &[u8]) -> bool {
    bytes.starts_with(b"SEGGER J-Link")
}

fn normalize_reader_buf_bytes(value: usize) -> usize {
    value.max(1)
}

async fn wait_backoff(shutdown: &CancellationToken, options: &ListenerOptions, attempts: u32) {
    if attempts == 0 {
        return;
    }

    let wait = cmp::min(
        options.reconnect.saturating_mul(attempts),
        options.reconnect_max,
    );

    tokio::select! {
        _ = shutdown.cancelled() => {}
        _ = tokio::time::sleep(wait) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_like_jlink_banner_matches_prefix() {
        assert!(looks_like_jlink_banner(
            b"SEGGER J-Link V9.16a - Real time terminal output\r\n"
        ));
        assert!(!looks_like_jlink_banner(b"\x00\x01\x02"));
    }

    #[test]
    fn reader_buffer_is_normalized() {
        assert_eq!(normalize_reader_buf_bytes(0), 1);
        assert_eq!(normalize_reader_buf_bytes(4096), 4096);
    }
}
