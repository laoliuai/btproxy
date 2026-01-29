use bytes::Bytes;
use common::error::{BtProxyError, Result};
use std::fs::File;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, info};

#[derive(Debug, Clone)]
pub struct BtLinkConfig {
    pub max_chunk: usize,
    pub queue_bound: usize,
    pub stats_interval: Option<Duration>,
}

impl Default for BtLinkConfig {
    fn default() -> Self {
        Self {
            max_chunk: 4096,
            queue_bound: 256,
            stats_interval: Some(Duration::from_secs(5)),
        }
    }
}

pub struct BtLink {
    pub tx: mpsc::Sender<Bytes>,
    pub rx: mpsc::Receiver<Bytes>,
}

impl BtLink {
    pub fn spawn<S>(stream: S, cfg: BtLinkConfig) -> Result<Self>
    where
        S: BtStream,
    {
        let mut reader = stream.try_clone()?;
        let mut writer = stream;
        let (tx_outgoing, mut rx_outgoing) = mpsc::channel::<Bytes>(cfg.queue_bound);
        let (tx_incoming, rx_incoming) = mpsc::channel::<Bytes>(cfg.queue_bound);

        let max_chunk = cfg.max_chunk;
        let tx_incoming_clone = tx_incoming.clone();

        let rx_bytes = Arc::new(AtomicU64::new(0));
        let tx_bytes = Arc::new(AtomicU64::new(0));
        let stats_interval = cfg.stats_interval;
        let rx_bytes_reader = Arc::clone(&rx_bytes);
        thread::spawn(move || {
            let mut buf = vec![0u8; max_chunk];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        debug!("btlink reader eof");
                        break;
                    }
                    Ok(n) => {
                        rx_bytes_reader.fetch_add(n as u64, Ordering::Relaxed);
                        if tx_incoming_clone
                            .blocking_send(Bytes::copy_from_slice(&buf[..n]))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(err) => {
                        debug!(?err, "btlink reader error");
                        break;
                    }
                }
            }
        });

        let tx_bytes_writer = Arc::clone(&tx_bytes);
        thread::spawn(move || {
            while let Some(chunk) = rx_outgoing.blocking_recv() {
                tx_bytes_writer.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                if let Err(err) = writer.write_all(&chunk) {
                    debug!(?err, "btlink writer error");
                    break;
                }
            }
        });

        if let Some(interval) = stats_interval {
            let rx_bytes_stats = Arc::clone(&rx_bytes);
            let tx_bytes_stats = Arc::clone(&tx_bytes);
            thread::spawn(move || {
                let mut last_rx = 0u64;
                let mut last_tx = 0u64;
                let mut last_at = Instant::now();
                loop {
                    thread::sleep(interval);
                    let now = Instant::now();
                    let elapsed = now.duration_since(last_at);
                    let rx_total = rx_bytes_stats.load(Ordering::Relaxed);
                    let tx_total = tx_bytes_stats.load(Ordering::Relaxed);
                    let rx_delta = rx_total.saturating_sub(last_rx);
                    let tx_delta = tx_total.saturating_sub(last_tx);
                    last_rx = rx_total;
                    last_tx = tx_total;
                    last_at = now;

                    let elapsed_secs = elapsed.as_secs_f64().max(0.001);
                    let rx_rate = rx_delta as f64 / elapsed_secs;
                    let tx_rate = tx_delta as f64 / elapsed_secs;
                    info!(
                        rx_bytes = rx_total,
                        tx_bytes = tx_total,
                        rx_bps = rx_rate,
                        tx_bps = tx_rate,
                        "btlink throughput"
                    );
                }
            });
        }

        Ok(Self {
            tx: tx_outgoing,
            rx: rx_incoming,
        })
    }
}

pub trait BtStream: Read + Write + Send + 'static {
    fn try_clone(&self) -> Result<Self>
    where
        Self: Sized;
}

impl BtStream for File {
    fn try_clone(&self) -> Result<Self> {
        Ok(self.try_clone()?)
    }
}

impl BtStream for TcpStream {
    fn try_clone(&self) -> Result<Self> {
        Ok(self.try_clone()?)
    }
}

pub fn unsupported<T>(msg: &str) -> Result<T> {
    Err(BtProxyError::Unsupported(msg.to_string()))
}
