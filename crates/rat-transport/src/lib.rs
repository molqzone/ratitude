use std::cmp;
use std::io;
use std::time::Duration;

use bytes::BytesMut;
use futures_util::StreamExt;
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
}

impl Default for ListenerOptions {
    fn default() -> Self {
        Self {
            reconnect: Duration::from_secs(1),
            reconnect_max: Duration::from_secs(30),
            dial_timeout: Duration::from_secs(5),
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
}

pub fn spawn_listener(
    shutdown: CancellationToken,
    addr: String,
    out: mpsc::Sender<Vec<u8>>,
    options: ListenerOptions,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut attempts: u32 = 0;

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

            match handle_connection(stream, &out, shutdown.clone()).await {
                Ok(()) => {
                    info!(%addr, "transport connection closed");
                }
                Err(err) => {
                    attempts = 1;
                    warn!(%addr, error = %err, "transport connection error");
                }
            }

            wait_backoff(&shutdown, &options, attempts).await;
        }
    })
}

async fn handle_connection(
    stream: TcpStream,
    out: &mpsc::Sender<Vec<u8>>,
    shutdown: CancellationToken,
) -> Result<(), io::Error> {
    let mut framed = FramedRead::new(stream, ZeroDelimitedFrameCodec::default());

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
