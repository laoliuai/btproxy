use crate::link::{unsupported, BtLink, BtLinkConfig};
use common::error::{BtProxyError, Result};
use std::fs::File;
use std::io;
use std::os::unix::io::{FromRawFd, RawFd};
use tracing::info;

const AF_BLUETOOTH: i32 = 31;
const BTPROTO_RFCOMM: i32 = 3;
const SOCK_STREAM: i32 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
struct SockAddrRc {
    rc_family: libc::sa_family_t,
    rc_bdaddr: [u8; 6],
    rc_channel: u8,
}

fn parse_bdaddr(addr: &str) -> Result<[u8; 6]> {
    let parts: Vec<&str> = addr.split(':').collect();
    if parts.len() != 6 {
        return Err(BtProxyError::Config("invalid bt addr".to_string()));
    }
    let mut bytes = [0u8; 6];
    for (i, part) in parts.iter().enumerate() {
        bytes[5 - i] = u8::from_str_radix(part, 16)
            .map_err(|_| BtProxyError::Config("invalid bt addr".to_string()))?;
    }
    Ok(bytes)
}

fn socket_fd() -> Result<RawFd> {
    let fd = unsafe { libc::socket(AF_BLUETOOTH, SOCK_STREAM, BTPROTO_RFCOMM) };
    if fd < 0 {
        return Err(BtProxyError::Io(io::Error::last_os_error()));
    }
    Ok(fd)
}

fn file_from_fd(fd: RawFd) -> File {
    unsafe { File::from_raw_fd(fd) }
}

pub async fn connect_linux_rfcomm(addr: &str, channel: u8, cfg: BtLinkConfig) -> Result<BtLink> {
    let fd = socket_fd()?;
    let bdaddr = parse_bdaddr(addr)?;
    let sockaddr = SockAddrRc {
        rc_family: AF_BLUETOOTH as libc::sa_family_t,
        rc_bdaddr: bdaddr,
        rc_channel: channel,
    };
    let ret = unsafe {
        libc::connect(
            fd,
            &sockaddr as *const SockAddrRc as *const libc::sockaddr,
            std::mem::size_of::<SockAddrRc>() as u32,
        )
    };
    if ret < 0 {
        return Err(BtProxyError::Io(io::Error::last_os_error()));
    }
    info!("rfcomm connected");
    Ok(BtLink::spawn(file_from_fd(fd), cfg)?)
}

pub async fn accept_linux_rfcomm(channel: u8, cfg: BtLinkConfig) -> Result<BtLink> {
    let fd = socket_fd()?;
    let sockaddr = SockAddrRc {
        rc_family: AF_BLUETOOTH as libc::sa_family_t,
        rc_bdaddr: [0; 6],
        rc_channel: channel,
    };
    let ret = unsafe {
        libc::bind(
            fd,
            &sockaddr as *const SockAddrRc as *const libc::sockaddr,
            std::mem::size_of::<SockAddrRc>() as u32,
        )
    };
    if ret < 0 {
        return Err(BtProxyError::Io(io::Error::last_os_error()));
    }
    if unsafe { libc::listen(fd, 1) } < 0 {
        return Err(BtProxyError::Io(io::Error::last_os_error()));
    }
    let client_fd = unsafe { libc::accept(fd, std::ptr::null_mut(), std::ptr::null_mut()) };
    if client_fd < 0 {
        return Err(BtProxyError::Io(io::Error::last_os_error()));
    }
    Ok(BtLink::spawn(file_from_fd(client_fd), cfg)?)
}

#[allow(dead_code)]
pub async fn connect_linux_rfcomm_by_uuid(_addr: &str, _uuid: &str) -> Result<BtLink> {
    unsupported("linux uuid sdp lookup not implemented")
}
