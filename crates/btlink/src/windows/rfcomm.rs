use crate::link::unsupported;
use crate::link::{BtLink, BtLinkConfig};
use common::error::Result;

pub async fn connect_windows_rfcomm(
    _addr: &str,
    _uuid: Option<&str>,
    _channel: Option<u8>,
    _cfg: BtLinkConfig,
) -> Result<BtLink> {
    unsupported("windows rfcomm not implemented in this build")
}

pub async fn accept_windows_rfcomm(_channel: u8, _cfg: BtLinkConfig) -> Result<BtLink> {
    unsupported("windows rfcomm not implemented in this build")
}
