use crate::link::{BtLink, BtLinkConfig};
use common::error::{BtProxyError, Result};
use std::io;
use std::net::TcpStream;
use std::os::windows::io::{FromRawSocket, RawSocket};
use std::sync::OnceLock;
use tracing::info;
use windows_sys::core::GUID;
use windows_sys::Win32::Devices::Bluetooth::{AF_BTH, BTHPROTO_RFCOMM, SOCKADDR_BTH};
use windows_sys::Win32::Networking::WinSock::{
    accept, bind, closesocket, connect, listen, socket, WSAGetLastError, WSAStartup,
    INVALID_SOCKET, SOCKADDR, SOCK_STREAM, SOCKET, SOCKET_ERROR, WSADATA,
};

static WSA_STARTUP_RESULT: OnceLock<io::Result<()>> = OnceLock::new();

fn ensure_wsa() -> Result<()> {
    let result = WSA_STARTUP_RESULT.get_or_init(|| unsafe {
        let mut data = std::mem::zeroed::<WSADATA>();
        let ret = WSAStartup(0x0202, &mut data);
        if ret == 0 {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(ret))
        }
    });
    match result {
        Ok(()) => Ok(()),
        Err(err) => Err(BtProxyError::Io(error_from_io(err))),
    }
}

fn error_from_io(err: &io::Error) -> io::Error {
    if let Some(code) = err.raw_os_error() {
        io::Error::from_raw_os_error(code)
    } else {
        io::Error::new(err.kind(), err.to_string())
    }
}

fn last_socket_error() -> BtProxyError {
    let code = unsafe { WSAGetLastError() };
    BtProxyError::Io(io::Error::from_raw_os_error(code))
}

fn parse_bt_addr(addr: &str) -> Result<u64> {
    let parts: Vec<&str> = addr.split(':').collect();
    if parts.len() != 6 {
        return Err(BtProxyError::Config("invalid bt addr".to_string()));
    }
    let mut bytes = [0u8; 6];
    for (idx, part) in parts.iter().enumerate() {
        bytes[5 - idx] = u8::from_str_radix(part, 16)
            .map_err(|_| BtProxyError::Config("invalid bt addr".to_string()))?;
    }
    let mut value: u64 = 0;
    for byte in bytes {
        value = (value << 8) | u64::from(byte);
    }
    Ok(value)
}

fn parse_uuid(uuid: &str) -> Result<GUID> {
    let parts: Vec<&str> = uuid.trim().split('-').collect();
    if parts.len() != 5 {
        return Err(BtProxyError::Config("invalid uuid".to_string()));
    }
    let data1 = u32::from_str_radix(parts[0], 16)
        .map_err(|_| BtProxyError::Config("invalid uuid".to_string()))?;
    let data2 = u16::from_str_radix(parts[1], 16)
        .map_err(|_| BtProxyError::Config("invalid uuid".to_string()))?;
    let data3 = u16::from_str_radix(parts[2], 16)
        .map_err(|_| BtProxyError::Config("invalid uuid".to_string()))?;
    if parts[3].len() != 4 || parts[4].len() != 12 {
        return Err(BtProxyError::Config("invalid uuid".to_string()));
    }
    let bytes4 = hex_to_bytes(parts[3])?;
    let bytes5 = hex_to_bytes(parts[4])?;
    if bytes4.len() != 2 || bytes5.len() != 6 {
        return Err(BtProxyError::Config("invalid uuid".to_string()));
    }
    let mut data4 = [0u8; 8];
    data4[..2].copy_from_slice(&bytes4);
    data4[2..].copy_from_slice(&bytes5);
    Ok(GUID {
        data1,
        data2,
        data3,
        data4,
    })
}

fn hex_to_bytes(value: &str) -> Result<Vec<u8>> {
    if value.len() % 2 != 0 {
        return Err(BtProxyError::Config("invalid uuid".to_string()));
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for idx in (0..value.len()).step_by(2) {
        let byte = u8::from_str_radix(&value[idx..idx + 2], 16)
            .map_err(|_| BtProxyError::Config("invalid uuid".to_string()))?;
        bytes.push(byte);
    }
    Ok(bytes)
}

fn null_guid() -> GUID {
    GUID {
        data1: 0,
        data2: 0,
        data3: 0,
        data4: [0u8; 8],
    }
}

fn socket_stream(socket: SOCKET) -> Result<TcpStream> {
    if socket == INVALID_SOCKET {
        return Err(last_socket_error());
    }
    let raw = socket as RawSocket;
    Ok(unsafe { TcpStream::from_raw_socket(raw) })
}

fn close_socket(socket: SOCKET) {
    if socket != INVALID_SOCKET {
        unsafe {
            closesocket(socket);
        }
    }
}

pub async fn connect_windows_rfcomm(
    addr: &str,
    uuid: Option<&str>,
    channel: Option<u8>,
    cfg: BtLinkConfig,
) -> Result<BtLink> {
    ensure_wsa()?;
    let target_addr = parse_bt_addr(addr)?;
    let mut sockaddr = SOCKADDR_BTH {
        addressFamily: AF_BTH as u16,
        btAddr: target_addr,
        serviceClassId: null_guid(),
        port: 0,
    };
    if let Some(channel) = channel {
        sockaddr.port = u32::from(channel);
    } else if let Some(uuid) = uuid {
        sockaddr.serviceClassId = parse_uuid(uuid)?;
    } else {
        return Err(BtProxyError::Config(
            "missing channel or uuid for windows rfcomm".to_string(),
        ));
    }
    let socket = unsafe { socket(AF_BTH as i32, SOCK_STREAM as i32, BTHPROTO_RFCOMM as i32) };
    if socket == INVALID_SOCKET {
        return Err(last_socket_error());
    }
    info!(
        bt_addr = %addr,
        channel = channel.map(u32::from),
        uuid = uuid.unwrap_or(""),
        "connecting rfcomm"
    );
    let ret = unsafe {
        connect(
            socket,
            &sockaddr as *const SOCKADDR_BTH as *const SOCKADDR,
            std::mem::size_of::<SOCKADDR_BTH>() as i32,
        )
    };
    if ret == SOCKET_ERROR {
        let err = unsafe { WSAGetLastError() };
        close_socket(socket);
        return Err(BtProxyError::Io(io::Error::from_raw_os_error(err)));
    }
    let stream = socket_stream(socket)?;
    info!("rfcomm connected");
    Ok(BtLink::spawn(stream, cfg)?)
}

pub async fn accept_windows_rfcomm(channel: u8, cfg: BtLinkConfig) -> Result<BtLink> {
    ensure_wsa()?;
    let socket = unsafe { socket(AF_BTH as i32, SOCK_STREAM as i32, BTHPROTO_RFCOMM as i32) };
    if socket == INVALID_SOCKET {
        return Err(last_socket_error());
    }
    info!(channel, "rfcomm listening");
    let sockaddr = SOCKADDR_BTH {
        addressFamily: AF_BTH as u16,
        btAddr: 0,
        serviceClassId: null_guid(),
        port: u32::from(channel),
    };
    let ret = unsafe {
        bind(
            socket,
            &sockaddr as *const SOCKADDR_BTH as *const SOCKADDR,
            std::mem::size_of::<SOCKADDR_BTH>() as i32,
        )
    };
    if ret == SOCKET_ERROR {
        close_socket(socket);
        return Err(last_socket_error());
    }
    if unsafe { listen(socket, 1) } == SOCKET_ERROR {
        close_socket(socket);
        return Err(last_socket_error());
    }
    let client_socket = unsafe { accept(socket, std::ptr::null_mut(), std::ptr::null_mut()) };
    close_socket(socket);
    if client_socket == INVALID_SOCKET {
        return Err(last_socket_error());
    }
    let stream = socket_stream(client_socket)?;
    info!("rfcomm accepted client");
    Ok(BtLink::spawn(stream, cfg)?)
}
