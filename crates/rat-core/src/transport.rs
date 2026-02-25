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
    pub reader_buf_bytes: usize,
}

impl Default for ListenerOptions {
    fn default() -> Self {
        Self {
            reconnect: Duration::from_secs(1),
            reconnect_max: Duration::from_secs(30),
            dial_timeout: Duration::from_secs(5),
            reader_buf_bytes: 65_536,
        }
    }
}

const JLINK_BANNER_PREFIX: &[u8] = b"SEGGER J-Link";
const JLINK_BANNER_MAX_BYTES: usize = 1024;
const MAX_FRAME_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum BannerStripState {
    #[default]
    Pending,
    Done,
}

#[derive(Debug, Default)]
struct ZeroDelimitedFrameCodec {
    banner_strip_state: BannerStripState,
}

impl Decoder for ZeroDelimitedFrameCodec {
    type Item = Vec<u8>;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if self.banner_strip_state == BannerStripState::Pending {
            match try_strip_initial_jlink_banner(src) {
                BannerStripOutcome::NeedMore => return Ok(None),
                BannerStripOutcome::Consumed => {
                    self.banner_strip_state = BannerStripState::Done;
                }
                BannerStripOutcome::Skip => {
                    self.banner_strip_state = BannerStripState::Done;
                }
            }
        }

        let Some(delimiter_index) = src.iter().position(|byte| *byte == 0) else {
            if src.len() > MAX_FRAME_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "frame buffer exceeds max bytes without delimiter: {} > {}",
                        src.len(),
                        MAX_FRAME_BYTES
                    ),
                ));
            }
            return Ok(None);
        };
        if delimiter_index > MAX_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "frame payload exceeds max bytes: {} > {}",
                    delimiter_index, MAX_FRAME_BYTES
                ),
            ));
        }

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

            match handle_connection(stream, &out, shutdown.clone(), reader_buf_bytes).await {
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
    stream: TcpStream,
    out: &mpsc::Sender<Vec<u8>>,
    shutdown: CancellationToken,
    reader_buf_bytes: usize,
) -> Result<(), io::Error> {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BannerStripOutcome {
    NeedMore,
    Consumed,
    Skip,
}

fn try_strip_initial_jlink_banner(src: &mut BytesMut) -> BannerStripOutcome {
    if src.is_empty() {
        return BannerStripOutcome::NeedMore;
    }

    if !looks_like_jlink_banner_prefix(src) {
        debug!("jlink banner strip skipped: non-banner prefix");
        return BannerStripOutcome::Skip;
    }

    let line_end = src.iter().position(|byte| *byte == b'\n');
    let delimiter_index = src.iter().position(|byte| *byte == 0);
    if let Some(delimiter) = delimiter_index {
        if line_end.map(|line| delimiter < line).unwrap_or(true) {
            debug!("jlink banner strip skipped: frame delimiter before banner line ending");
            return BannerStripOutcome::Skip;
        }
    }

    if let Some(line_end) = line_end {
        if line_end + 1 > JLINK_BANNER_MAX_BYTES {
            debug!(
                line_bytes = line_end + 1,
                "jlink banner strip skipped: banner line exceeds safe limit"
            );
            return BannerStripOutcome::Skip;
        }
        let _ = src.split_to(line_end + 1);
        debug!(line_bytes = line_end + 1, "jlink banner line consumed");
        return BannerStripOutcome::Consumed;
    }

    if src.len() > JLINK_BANNER_MAX_BYTES {
        debug!(
            buffered_bytes = src.len(),
            "jlink banner strip skipped: no newline within safe limit"
        );
        return BannerStripOutcome::Skip;
    }

    BannerStripOutcome::NeedMore
}

fn looks_like_jlink_banner_prefix(bytes: &[u8]) -> bool {
    if bytes.len() >= JLINK_BANNER_PREFIX.len() {
        bytes.starts_with(JLINK_BANNER_PREFIX)
    } else {
        JLINK_BANNER_PREFIX.starts_with(bytes)
    }
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
    use bytes::BufMut;

    use super::*;

    #[test]
    fn looks_like_jlink_banner_matches_prefix() {
        assert!(looks_like_jlink_banner_prefix(
            b"SEGGER J-Link V9.16a - Real time terminal output\r\n"
        ));
        assert!(!looks_like_jlink_banner_prefix(b"\x00\x01\x02"));
        assert!(looks_like_jlink_banner_prefix(b"SEGGER J-"));
    }

    #[test]
    fn reader_buffer_is_normalized() {
        assert_eq!(normalize_reader_buf_bytes(0), 1);
        assert_eq!(normalize_reader_buf_bytes(4096), 4096);
    }

    #[test]
    fn codec_strips_jlink_banner_then_decodes_first_frame() {
        let mut codec = ZeroDelimitedFrameCodec::default();
        let mut src = BytesMut::from(
            &b"SEGGER J-Link V9.16a - Real time terminal output\r\nabc\x00rest\x00"[..],
        );

        let first = codec.decode(&mut src).expect("decode first");
        let second = codec.decode(&mut src).expect("decode second");

        assert_eq!(first, Some(b"abc".to_vec()));
        assert_eq!(second, Some(b"rest".to_vec()));
    }

    #[test]
    fn codec_keeps_non_banner_payload_unchanged() {
        let mut codec = ZeroDelimitedFrameCodec::default();
        let mut src = BytesMut::from(&b"payload\x00"[..]);

        let frame = codec.decode(&mut src).expect("decode");
        assert_eq!(frame, Some(b"payload".to_vec()));
    }

    #[test]
    fn codec_handles_partial_banner_prefix_across_chunks() {
        let mut codec = ZeroDelimitedFrameCodec::default();
        let mut src = BytesMut::from(&b"SEGGER J-"[..]);

        let first = codec.decode(&mut src).expect("decode first");
        assert!(first.is_none());

        src.put_slice(b"Link V9.16a - Real time terminal output\r\nxyz\x00");
        let second = codec.decode(&mut src).expect("decode second");
        assert_eq!(second, Some(b"xyz".to_vec()));
    }

    #[test]
    fn codec_fallbacks_when_banner_line_exceeds_limit() {
        let mut codec = ZeroDelimitedFrameCodec::default();
        let mut raw = Vec::new();
        raw.extend_from_slice(JLINK_BANNER_PREFIX);
        raw.extend(std::iter::repeat_n(b'A', JLINK_BANNER_MAX_BYTES + 32));
        raw.push(0);
        let mut src = BytesMut::from(raw.as_slice());

        let frame = codec.decode(&mut src).expect("decode");
        let frame = frame.expect("frame");
        assert!(frame.starts_with(JLINK_BANNER_PREFIX));
        assert_eq!(
            frame.len(),
            JLINK_BANNER_PREFIX.len() + JLINK_BANNER_MAX_BYTES + 32
        );
    }

    #[test]
    fn codec_rejects_buffer_growth_without_delimiter() {
        let mut codec = ZeroDelimitedFrameCodec::default();
        let raw = vec![b'A'; MAX_FRAME_BYTES + 1];
        let mut src = BytesMut::from(raw.as_slice());

        let err = codec.decode(&mut src).expect_err("oversized buffer");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("exceeds max bytes"));
    }

    #[test]
    fn codec_rejects_oversized_frame_payload() {
        let mut codec = ZeroDelimitedFrameCodec::default();
        let mut raw = vec![b'A'; MAX_FRAME_BYTES + 1];
        raw.push(0);
        let mut src = BytesMut::from(raw.as_slice());

        let err = codec.decode(&mut src).expect_err("oversized frame");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("frame payload exceeds max bytes"));
    }
}
