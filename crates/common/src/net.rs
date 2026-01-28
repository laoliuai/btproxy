use crate::error::{BtProxyError, Result};
use tokio::io::{AsyncRead, AsyncReadExt};

pub async fn read_until_double_crlf<R: AsyncRead + Unpin>(
    reader: &mut R,
    max: usize,
) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        if buf.len() >= max {
            return Err(BtProxyError::Protocol("header too large".to_string()));
        }
        let n = reader.read(&mut tmp).await?;
        if n == 0 {
            return Err(BtProxyError::Protocol("unexpected eof".to_string()));
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            return Ok(buf);
        }
    }
}

pub struct Backoff {
    current: u64,
    max: u64,
}

impl Backoff {
    pub fn new(initial_ms: u64, max_ms: u64) -> Self {
        Self {
            current: initial_ms,
            max: max_ms,
        }
    }

    pub fn next_delay(&mut self) -> u64 {
        let delay = self.current;
        self.current = (self.current * 2).min(self.max);
        delay
    }

    pub fn reset(&mut self, initial_ms: u64) {
        self.current = initial_ms;
    }
}
