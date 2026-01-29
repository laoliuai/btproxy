use crate::codec::try_decode;
use crate::frame::{Frame, TargetAddr};
use crate::handshake::{build_hello, build_hello_ack, verify_hmac};
use crate::keepalive::keepalive_task;
use crate::stream::MuxStream;
use bytes::{Bytes, BytesMut};
use common::error::{BtProxyError, Result};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Copy)]
pub enum Role {
    Client,
    Server,
}

#[derive(Debug, Clone)]
pub struct MuxConfig {
    pub max_frame: usize,
    pub keepalive_ms: u32,
    pub psk: Option<Vec<u8>>,
}

impl Default for MuxConfig {
    fn default() -> Self {
        Self {
            max_frame: 65536,
            keepalive_ms: 10_000,
            psk: None,
        }
    }
}

#[derive(Clone)]
pub struct MuxSession {
    inner: Arc<InnerSession>,
}

struct InnerSession {
    outgoing: mpsc::Sender<Frame>,
    incoming: Mutex<mpsc::Receiver<(TargetAddr, MuxStream)>>,
    streams: Arc<Mutex<std::collections::HashMap<u32, mpsc::Sender<Bytes>>>>,
    pending: Arc<Mutex<std::collections::HashMap<u32, oneshot::Sender<Result<()>>>>>,
    next_stream_id: Mutex<u32>,
    _tasks: Vec<JoinHandle<()>>,
}

impl MuxSession {
    pub async fn start(link: btlink::BtLink, cfg: MuxConfig, role: Role) -> Result<Self> {
        let (tx_frame, mut rx_frame) = mpsc::channel::<Frame>(cfg.max_frame / 1024 + 32);
        let (tx_open, rx_open) = mpsc::channel::<(TargetAddr, MuxStream)>(128);
        let psk = cfg.psk.clone();

        let (link_tx, mut link_rx) = (link.tx, link.rx);

        let hello = build_hello(cfg.max_frame as u32, cfg.keepalive_ms, psk.as_deref());
        link_tx
            .send(hello.encode()?.into())
            .await
            .map_err(|_| BtProxyError::Protocol("failed to send hello".to_string()))?;

        let mut buffer = BytesMut::new();
        let mut got_handshake = false;
        while !got_handshake {
            let chunk = link_rx
                .recv()
                .await
                .ok_or_else(|| BtProxyError::Protocol("handshake eof".to_string()))?;
            buffer.extend_from_slice(&chunk);
            if let Some(frame) = try_decode(&mut buffer, cfg.max_frame)? {
                match frame {
                    Frame::Hello(frame) => {
                        verify_hmac(psk.as_deref(), &frame)?;
                        let ack = build_hello_ack(
                            cfg.max_frame as u32,
                            cfg.keepalive_ms,
                            psk.as_deref(),
                            frame.nonce,
                        );
                        link_tx.send(ack.encode()?.into()).await.map_err(|_| {
                            BtProxyError::Protocol("failed to send hello ack".to_string())
                        })?;
                        got_handshake = true;
                    }
                    Frame::HelloAck(frame) => {
                        verify_hmac(psk.as_deref(), &frame)?;
                        got_handshake = true;
                    }
                    _ => {
                        warn!(?frame, "unexpected frame before handshake");
                    }
                }
            }
        }

        let streams: Arc<Mutex<std::collections::HashMap<u32, mpsc::Sender<Bytes>>>> =
            Arc::new(Mutex::new(std::collections::HashMap::new()));
        let pending: Arc<Mutex<std::collections::HashMap<u32, oneshot::Sender<Result<()>>>>> =
            Arc::new(Mutex::new(std::collections::HashMap::new()));
        let next_stream_id = Mutex::new(1u32);

        let streams_clone = Arc::clone(&streams);
        let pending_clone = Arc::clone(&pending);
        let tx_open_clone = tx_open.clone();
        let mut buffer = buffer;

        let tx_frame_for_read = tx_frame.clone();
        let read_task = tokio::spawn(async move {
            loop {
                match link_rx.recv().await {
                    Some(chunk) => {
                        buffer.extend_from_slice(&chunk);
                        loop {
                            match try_decode(&mut buffer, cfg.max_frame) {
                                Ok(Some(frame)) => match frame {
                                    Frame::Open { stream_id, target } => {
                                        let (tx_stream, rx_stream) = mpsc::channel(128);
                                        streams_clone.lock().await.insert(stream_id, tx_stream);
                                        let stream = MuxStream::new(
                                            stream_id,
                                            tx_frame_for_read.clone(),
                                            rx_stream,
                                        );
                                        if tx_open_clone.send((target, stream)).await.is_err() {
                                            break;
                                        }
                                    }
                                    Frame::OpenOk { stream_id } => {
                                        if let Some(tx) =
                                            pending_clone.lock().await.remove(&stream_id)
                                        {
                                            let _ = tx.send(Ok(()));
                                        }
                                    }
                                    Frame::OpenErr {
                                        stream_id, message, ..
                                    } => {
                                        if let Some(tx) =
                                            pending_clone.lock().await.remove(&stream_id)
                                        {
                                            let _ = tx.send(Err(BtProxyError::Protocol(message)));
                                        }
                                    }
                                    Frame::Data { stream_id, payload } => {
                                        if let Some(tx) = streams_clone.lock().await.get(&stream_id)
                                        {
                                            let _ = tx.send(payload).await;
                                        }
                                    }
                                    Frame::Fin { stream_id } | Frame::Rst { stream_id, .. } => {
                                        streams_clone.lock().await.remove(&stream_id);
                                    }
                                    Frame::Ping { nonce } => {
                                        let _ = tx_frame_for_read.send(Frame::Pong { nonce }).await;
                                    }
                                    Frame::Pong { .. } => {}
                                    Frame::Hello(_) | Frame::HelloAck(_) => {}
                                },
                                Ok(None) => break,
                                Err(err) => {
                                    debug!(?err, "decode error");
                                    break;
                                }
                            }
                        }
                    }
                    None => break,
                }
            }
        });

        let write_task = tokio::spawn(async move {
            while let Some(frame) = rx_frame.recv().await {
                match frame.encode() {
                    Ok(bytes) => {
                        if link_tx.send(bytes).await.is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        debug!(?err, "encode error");
                    }
                }
            }
        });

        let keepalive_handle = tokio::spawn(keepalive_task(tx_frame.clone(), cfg.keepalive_ms));

        let tasks = vec![read_task, write_task, keepalive_handle];

        let outgoing = tx_frame;

        info!(?role, "mux session started");

        Ok(Self {
            inner: Arc::new(InnerSession {
                outgoing,
                incoming: Mutex::new(rx_open),
                streams: Arc::clone(&streams),
                pending: Arc::clone(&pending),
                next_stream_id,
                _tasks: tasks,
            }),
        })
    }

    pub async fn open_stream(&self, target: TargetAddr) -> Result<MuxStream> {
        let mut id_guard = self.inner.next_stream_id.lock().await;
        let stream_id = *id_guard;
        *id_guard = id_guard.wrapping_add(1);
        drop(id_guard);

        let (tx_stream, rx_stream) = mpsc::channel(128);
        self.inner.streams.lock().await.insert(stream_id, tx_stream);

        let (tx_pending, rx_pending) = oneshot::channel();
        self.inner
            .pending
            .lock()
            .await
            .insert(stream_id, tx_pending);

        self.inner
            .outgoing
            .send(Frame::Open { stream_id, target })
            .await
            .map_err(|_| BtProxyError::Protocol("failed to send open".to_string()))?;

        match rx_pending.await {
            Ok(Ok(())) => Ok(MuxStream::new(
                stream_id,
                self.inner.outgoing.clone(),
                rx_stream,
            )),
            Ok(Err(err)) => {
                self.inner.streams.lock().await.remove(&stream_id);
                Err(err)
            }
            Err(_) => {
                self.inner.streams.lock().await.remove(&stream_id);
                Err(BtProxyError::Protocol("open canceled".to_string()))
            }
        }
    }

    pub async fn accept_stream(&self) -> Option<(TargetAddr, MuxStream)> {
        let mut rx = self.inner.incoming.lock().await;
        rx.recv().await
    }

    pub async fn send_open_ok(&self, stream_id: u32) -> Result<()> {
        self.inner
            .outgoing
            .send(Frame::OpenOk { stream_id })
            .await
            .map_err(|_| BtProxyError::Protocol("open ok send failed".to_string()))
    }

    pub async fn send_open_err(&self, stream_id: u32, code: u16, message: &str) -> Result<()> {
        self.inner.streams.lock().await.remove(&stream_id);
        self.inner.pending.lock().await.remove(&stream_id);
        self.inner
            .outgoing
            .send(Frame::OpenErr {
                stream_id,
                code,
                message: message.to_string(),
            })
            .await
            .map_err(|_| BtProxyError::Protocol("open err send failed".to_string()))
    }

    pub async fn send_rst(&self, stream_id: u32, code: u16) -> Result<()> {
        self.inner.streams.lock().await.remove(&stream_id);
        self.inner.pending.lock().await.remove(&stream_id);
        self.inner
            .outgoing
            .send(Frame::Rst { stream_id, code })
            .await
            .map_err(|_| BtProxyError::Protocol("rst send failed".to_string()))
    }
}
