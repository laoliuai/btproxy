use bytes::{BufMut, Bytes, BytesMut};
use common::error::{BtProxyError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameType {
    Hello = 0x01,
    HelloAck = 0x02,
    Open = 0x10,
    OpenOk = 0x11,
    OpenErr = 0x12,
    Data = 0x20,
    Fin = 0x21,
    Rst = 0x22,
    Ping = 0x30,
    Pong = 0x31,
}

#[derive(Debug, Clone)]
pub struct HelloFrame {
    pub version: u16,
    pub flags: u16,
    pub max_frame: u32,
    pub keepalive_ms: u32,
    pub nonce: u64,
    pub hmac: Option<[u8; 32]>,
}

#[derive(Debug, Clone)]
pub enum Frame {
    Hello(HelloFrame),
    HelloAck(HelloFrame),
    Open {
        stream_id: u32,
        target: TargetAddr,
    },
    OpenOk {
        stream_id: u32,
    },
    OpenErr {
        stream_id: u32,
        code: u16,
        message: String,
    },
    Data {
        stream_id: u32,
        payload: Bytes,
    },
    Fin {
        stream_id: u32,
    },
    Rst {
        stream_id: u32,
        code: u16,
    },
    Ping {
        nonce: u64,
    },
    Pong {
        nonce: u64,
    },
}

#[derive(Debug, Clone)]
pub enum TargetAddr {
    Domain(String, u16),
    IpV4([u8; 4], u16),
    IpV6([u8; 16], u16),
}

impl Frame {
    pub fn encode(&self) -> Result<Bytes> {
        let mut payload = BytesMut::new();
        let frame_type = match self {
            Frame::Hello(frame) => {
                payload.put_u16(frame.version);
                payload.put_u16(frame.flags);
                payload.put_u32(frame.max_frame);
                payload.put_u32(frame.keepalive_ms);
                payload.put_u64(frame.nonce);
                if let Some(hmac) = frame.hmac {
                    payload.extend_from_slice(&hmac);
                }
                FrameType::Hello
            }
            Frame::HelloAck(frame) => {
                payload.put_u16(frame.version);
                payload.put_u16(frame.flags);
                payload.put_u32(frame.max_frame);
                payload.put_u32(frame.keepalive_ms);
                payload.put_u64(frame.nonce);
                if let Some(hmac) = frame.hmac {
                    payload.extend_from_slice(&hmac);
                }
                FrameType::HelloAck
            }
            Frame::Open { stream_id, target } => {
                payload.put_u32(*stream_id);
                match target {
                    TargetAddr::Domain(host, port) => {
                        payload.put_u8(1);
                        payload.put_u16(host.len() as u16);
                        payload.extend_from_slice(host.as_bytes());
                        payload.put_u16(*port);
                    }
                    TargetAddr::IpV4(addr, port) => {
                        payload.put_u8(2);
                        payload.extend_from_slice(addr);
                        payload.put_u16(*port);
                    }
                    TargetAddr::IpV6(addr, port) => {
                        payload.put_u8(3);
                        payload.extend_from_slice(addr);
                        payload.put_u16(*port);
                    }
                }
                FrameType::Open
            }
            Frame::OpenOk { stream_id } => {
                payload.put_u32(*stream_id);
                FrameType::OpenOk
            }
            Frame::OpenErr {
                stream_id,
                code,
                message,
            } => {
                payload.put_u32(*stream_id);
                payload.put_u16(*code);
                payload.put_u16(message.len() as u16);
                payload.extend_from_slice(message.as_bytes());
                FrameType::OpenErr
            }
            Frame::Data {
                stream_id,
                payload: data,
            } => {
                payload.put_u32(*stream_id);
                payload.put_u16(data.len() as u16);
                payload.extend_from_slice(data);
                FrameType::Data
            }
            Frame::Fin { stream_id } => {
                payload.put_u32(*stream_id);
                FrameType::Fin
            }
            Frame::Rst { stream_id, code } => {
                payload.put_u32(*stream_id);
                payload.put_u16(*code);
                FrameType::Rst
            }
            Frame::Ping { nonce } => {
                payload.put_u64(*nonce);
                FrameType::Ping
            }
            Frame::Pong { nonce } => {
                payload.put_u64(*nonce);
                FrameType::Pong
            }
        };
        let total_len = 1 + payload.len();
        let mut buf = BytesMut::with_capacity(4 + total_len);
        buf.put_u32(total_len as u32);
        buf.put_u8(frame_type as u8);
        buf.extend_from_slice(&payload);
        Ok(buf.freeze())
    }

    pub fn decode(frame_type: u8, payload: &[u8]) -> Result<Frame> {
        let mut cursor = std::io::Cursor::new(payload);
        match frame_type {
            0x01 | 0x02 => {
                if payload.len() < 20 {
                    return Err(BtProxyError::Protocol("hello too short".to_string()));
                }
                use bytes::Buf;
                let version = cursor.get_u16();
                let flags = cursor.get_u16();
                let max_frame = cursor.get_u32();
                let keepalive_ms = cursor.get_u32();
                let nonce = cursor.get_u64();
                let hmac = if cursor.remaining() >= 32 {
                    let mut buf = [0u8; 32];
                    cursor.copy_to_slice(&mut buf);
                    Some(buf)
                } else {
                    None
                };
                let frame = HelloFrame {
                    version,
                    flags,
                    max_frame,
                    keepalive_ms,
                    nonce,
                    hmac,
                };
                if frame_type == 0x01 {
                    Ok(Frame::Hello(frame))
                } else {
                    Ok(Frame::HelloAck(frame))
                }
            }
            0x10 => {
                use bytes::Buf;
                let stream_id = cursor.get_u32();
                let addr_type = cursor.get_u8();
                let target = match addr_type {
                    1 => {
                        let len = cursor.get_u16() as usize;
                        let mut host_bytes = vec![0u8; len];
                        cursor.copy_to_slice(&mut host_bytes);
                        let port = cursor.get_u16();
                        TargetAddr::Domain(String::from_utf8_lossy(&host_bytes).to_string(), port)
                    }
                    2 => {
                        let mut ip = [0u8; 4];
                        cursor.copy_to_slice(&mut ip);
                        let port = cursor.get_u16();
                        TargetAddr::IpV4(ip, port)
                    }
                    3 => {
                        let mut ip = [0u8; 16];
                        cursor.copy_to_slice(&mut ip);
                        let port = cursor.get_u16();
                        TargetAddr::IpV6(ip, port)
                    }
                    _ => {
                        return Err(BtProxyError::Protocol("invalid addr type".to_string()));
                    }
                };
                Ok(Frame::Open { stream_id, target })
            }
            0x11 => {
                use bytes::Buf;
                let stream_id = cursor.get_u32();
                Ok(Frame::OpenOk { stream_id })
            }
            0x12 => {
                use bytes::Buf;
                let stream_id = cursor.get_u32();
                let code = cursor.get_u16();
                let msg_len = cursor.get_u16() as usize;
                let mut msg_bytes = vec![0u8; msg_len];
                cursor.copy_to_slice(&mut msg_bytes);
                let message = String::from_utf8_lossy(&msg_bytes).to_string();
                Ok(Frame::OpenErr {
                    stream_id,
                    code,
                    message,
                })
            }
            0x20 => {
                use bytes::Buf;
                let stream_id = cursor.get_u32();
                let data_len = cursor.get_u16() as usize;
                let mut data = vec![0u8; data_len];
                cursor.copy_to_slice(&mut data);
                Ok(Frame::Data {
                    stream_id,
                    payload: Bytes::from(data),
                })
            }
            0x21 => {
                use bytes::Buf;
                let stream_id = cursor.get_u32();
                Ok(Frame::Fin { stream_id })
            }
            0x22 => {
                use bytes::Buf;
                let stream_id = cursor.get_u32();
                let code = cursor.get_u16();
                Ok(Frame::Rst { stream_id, code })
            }
            0x30 => {
                use bytes::Buf;
                let nonce = cursor.get_u64();
                Ok(Frame::Ping { nonce })
            }
            0x31 => {
                use bytes::Buf;
                let nonce = cursor.get_u64();
                Ok(Frame::Pong { nonce })
            }
            _ => Err(BtProxyError::Protocol("unknown frame type".to_string())),
        }
    }
}
