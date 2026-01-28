use bytes::{BufMut, BytesMut};
use common::error::{BtProxyError, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::debug;

pub async fn connect_via_socks5(
    proxy: &str,
    username: Option<&str>,
    password: Option<&str>,
    host: &str,
    port: u16,
) -> Result<TcpStream> {
    let mut stream = TcpStream::connect(proxy).await?;
    let mut methods = vec![0x00u8];
    if username.is_some() {
        methods.push(0x02);
    }
    let mut greeting = BytesMut::with_capacity(2 + methods.len());
    greeting.put_u8(0x05);
    greeting.put_u8(methods.len() as u8);
    greeting.extend_from_slice(&methods);
    stream.write_all(&greeting).await?;

    let mut resp = [0u8; 2];
    stream.read_exact(&mut resp).await?;
    if resp[0] != 0x05 {
        return Err(BtProxyError::Protocol("invalid socks version".to_string()));
    }
    match resp[1] {
        0x00 => {}
        0x02 => {
            let user =
                username.ok_or_else(|| BtProxyError::Auth("username required".to_string()))?;
            let pass =
                password.ok_or_else(|| BtProxyError::Auth("password required".to_string()))?;
            let mut auth = BytesMut::new();
            auth.put_u8(0x01);
            auth.put_u8(user.len() as u8);
            auth.extend_from_slice(user.as_bytes());
            auth.put_u8(pass.len() as u8);
            auth.extend_from_slice(pass.as_bytes());
            stream.write_all(&auth).await?;
            let mut auth_resp = [0u8; 2];
            stream.read_exact(&mut auth_resp).await?;
            if auth_resp[1] != 0x00 {
                return Err(BtProxyError::Auth("socks auth failed".to_string()));
            }
        }
        _ => {
            return Err(BtProxyError::Protocol(
                "no acceptable auth method".to_string(),
            ));
        }
    }

    let mut request = BytesMut::new();
    request.put_u8(0x05);
    request.put_u8(0x01);
    request.put_u8(0x00);
    request.put_u8(0x03);
    request.put_u8(host.len() as u8);
    request.extend_from_slice(host.as_bytes());
    request.put_u16(port);
    stream.write_all(&request).await?;

    let mut header = [0u8; 4];
    stream.read_exact(&mut header).await?;
    if header[1] != 0x00 {
        debug!(code = header[1], "socks connect error");
        return Err(BtProxyError::Protocol("socks connect failed".to_string()));
    }
    let atyp = header[3];
    let addr_len = match atyp {
        0x01 => 4,
        0x03 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await?;
            len[0] as usize
        }
        0x04 => 16,
        _ => return Err(BtProxyError::Protocol("invalid atyp".to_string())),
    };
    let mut skip = vec![0u8; addr_len + 2];
    stream.read_exact(&mut skip).await?;
    Ok(stream)
}
