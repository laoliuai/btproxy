use crate::frame::Frame;
use bytes::Bytes;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

#[derive(Clone)]
pub struct MuxStream {
    pub stream_id: u32,
    outbound: mpsc::Sender<Frame>,
    inbound: Arc<Mutex<mpsc::Receiver<Bytes>>>,
}

impl MuxStream {
    pub fn new(
        stream_id: u32,
        outbound: mpsc::Sender<Frame>,
        inbound: mpsc::Receiver<Bytes>,
    ) -> Self {
        Self {
            stream_id,
            outbound,
            inbound: Arc::new(Mutex::new(inbound)),
        }
    }

    pub async fn send_data(&self, data: Bytes) -> Result<(), mpsc::error::SendError<Frame>> {
        self.outbound
            .send(Frame::Data {
                stream_id: self.stream_id,
                payload: data,
            })
            .await
    }

    pub async fn send_fin(&self) -> Result<(), mpsc::error::SendError<Frame>> {
        self.outbound
            .send(Frame::Fin {
                stream_id: self.stream_id,
            })
            .await
    }

    pub async fn recv_data(&self) -> Option<Bytes> {
        let mut rx = self.inbound.lock().await;
        rx.recv().await
    }
}
