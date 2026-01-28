use crate::frame::Frame;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};

pub async fn keepalive_task(tx: mpsc::Sender<Frame>, interval_ms: u32) {
    let mut ticker = interval(Duration::from_millis(interval_ms as u64));
    loop {
        ticker.tick().await;
        let nonce = rand::random::<u64>();
        if tx.send(Frame::Ping { nonce }).await.is_err() {
            break;
        }
    }
}
