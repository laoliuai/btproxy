use crate::frame::Frame;
use bytes::{Buf, BytesMut};
use common::error::{BtProxyError, Result};

pub fn try_decode(buffer: &mut BytesMut, max_frame: usize) -> Result<Option<Frame>> {
    if buffer.len() < 4 {
        return Ok(None);
    }
    let mut length_buf = &buffer[..4];
    let len = length_buf.get_u32() as usize;
    if len > max_frame {
        return Err(BtProxyError::Protocol("frame too large".to_string()));
    }
    if buffer.len() < 4 + len {
        return Ok(None);
    }
    buffer.advance(4);
    let frame_type = buffer.get_u8();
    let payload_len = len - 1;
    let payload = buffer.split_to(payload_len);
    Frame::decode(frame_type, &payload).map(Some)
}
