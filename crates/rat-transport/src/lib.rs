use std::cmp;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

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
                _ => {
                    attempts = attempts.saturating_add(1);
                    wait_backoff(&shutdown, &options, attempts).await;
                    continue;
                }
            };

            attempts = 0;
            let result = handle_connection(stream, &out, shutdown.clone()).await;
            if result.is_err() {
                attempts = 1;
            }
            wait_backoff(&shutdown, &options, attempts).await;
        }
    })
}

async fn handle_connection(
    mut stream: TcpStream,
    out: &mpsc::Sender<Vec<u8>>,
    shutdown: CancellationToken,
) -> Result<(), std::io::Error> {
    let mut chunk = [0u8; 4096];
    let mut frame = Vec::with_capacity(512);

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                return Ok(());
            }
            read_result = stream.read(&mut chunk) => {
                let read = read_result?;
                if read == 0 {
                    return Ok(());
                }

                for byte in &chunk[..read] {
                    if *byte == 0 {
                        if !frame.is_empty() {
                            if out.send(frame.clone()).await.is_err() {
                                return Ok(());
                            }
                            frame.clear();
                        }
                    } else {
                        frame.push(*byte);
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
