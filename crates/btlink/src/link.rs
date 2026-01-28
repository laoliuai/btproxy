use bytes::Bytes;
use common::error::{BtProxyError, Result};
use std::fs::File;
use std::io::{Read, Write};
use std::thread;
use tokio::sync::mpsc;
use tracing::debug;

#[derive(Debug, Clone)]
pub struct BtLinkConfig {
    pub max_chunk: usize,
    pub queue_bound: usize,
}

impl Default for BtLinkConfig {
    fn default() -> Self {
        Self {
            max_chunk: 4096,
            queue_bound: 256,
        }
    }
}

pub struct BtLink {
    pub tx: mpsc::Sender<Bytes>,
    pub rx: mpsc::Receiver<Bytes>,
}

impl BtLink {
    pub fn spawn(stream: File, cfg: BtLinkConfig) -> Result<Self> {
        let mut reader = stream.try_clone()?;
        let mut writer = stream;
        let (tx_outgoing, mut rx_outgoing) = mpsc::channel::<Bytes>(cfg.queue_bound);
        let (tx_incoming, rx_incoming) = mpsc::channel::<Bytes>(cfg.queue_bound);

        let max_chunk = cfg.max_chunk;
        let tx_incoming_clone = tx_incoming.clone();

        thread::spawn(move || {
            let mut buf = vec![0u8; max_chunk];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        debug!("btlink reader eof");
                        break;
                    }
                    Ok(n) => {
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

        thread::spawn(move || {
            while let Some(chunk) = rx_outgoing.blocking_recv() {
                if let Err(err) = writer.write_all(&chunk) {
                    debug!(?err, "btlink writer error");
                    break;
                }
            }
        });

        Ok(Self {
            tx: tx_outgoing,
            rx: rx_incoming,
        })
    }
}

pub fn unsupported<T>(msg: &str) -> Result<T> {
    Err(BtProxyError::Unsupported(msg.to_string()))
}
