use crate::frame::{Frame, HelloFrame};
use common::error::{BtProxyError, Result};
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;

const HMAC_LABEL: &[u8] = b"btproxy-v1";

pub fn build_hello(max_frame: u32, keepalive_ms: u32, psk: Option<&[u8]>) -> Frame {
    let mut rng = rand::thread_rng();
    let nonce = rng.next_u64();
    let hmac = psk.map(|key| compute_hmac(key, nonce));
    Frame::Hello(HelloFrame {
        version: 1,
        flags: if psk.is_some() { 1 } else { 0 },
        max_frame,
        keepalive_ms,
        nonce,
        hmac,
    })
}

pub fn build_hello_ack(max_frame: u32, keepalive_ms: u32, psk: Option<&[u8]>, nonce: u64) -> Frame {
    let hmac = psk.map(|key| compute_hmac(key, nonce));
    Frame::HelloAck(HelloFrame {
        version: 1,
        flags: if psk.is_some() { 1 } else { 0 },
        max_frame,
        keepalive_ms,
        nonce,
        hmac,
    })
}

pub fn verify_hmac(psk: Option<&[u8]>, frame: &HelloFrame) -> Result<()> {
    if let Some(key) = psk {
        let expected = compute_hmac(key, frame.nonce);
        if frame.hmac != Some(expected) {
            return Err(BtProxyError::Auth("invalid hmac".to_string()));
        }
    }
    Ok(())
}

fn compute_hmac(key: &[u8], nonce: u64) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("hmac key");
    mac.update(&nonce.to_be_bytes());
    mac.update(HMAC_LABEL);
    let result = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}
