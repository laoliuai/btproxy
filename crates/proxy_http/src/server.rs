use bytes::Bytes;
use common::error::{BtProxyError, Result};
use common::read_until_double_crlf;
use httparse::Request;
use mux::{MuxSession, MuxStream, TargetAddr};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{info, warn};
use url::Url;

pub async fn run_http_proxy(listen: &str, session: MuxSession) -> Result<()> {
    let listener = TcpListener::bind(listen).await?;
    info!("http proxy listening on {}", listen);
    loop {
        let (stream, addr) = listener.accept().await?;
        let session = session.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_client(stream, session).await {
                warn!(?addr, ?err, "client error");
            }
        });
    }
}

async fn handle_client(mut stream: TcpStream, session: MuxSession) -> Result<()> {
    let header = read_until_double_crlf(&mut stream, 64 * 1024).await?;
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut req = Request::new(&mut headers);
    let status = req
        .parse(&header)
        .map_err(|e| BtProxyError::Protocol(e.to_string()))?;
    if status.is_partial() {
        return Err(BtProxyError::Protocol("partial request".to_string()));
    }
    let method = req
        .method
        .ok_or_else(|| BtProxyError::Protocol("no method".to_string()))?;
    let path = req
        .path
        .ok_or_else(|| BtProxyError::Protocol("no path".to_string()))?;

    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = parse_connect_target(path)?;
        let target = TargetAddr::Domain(host.clone(), port);
        let mux_stream = session.open_stream(target).await?;
        stream
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await?;
        tunnel(stream, mux_stream).await?;
        return Ok(());
    }

    let url = Url::parse(path).map_err(|e| BtProxyError::Protocol(e.to_string()))?;
    let host = url
        .host_str()
        .ok_or_else(|| BtProxyError::Protocol("missing host".to_string()))?
        .to_string();
    let port = url.port_or_known_default().unwrap_or(80) as u16;
    let mut origin = url.path().to_string();
    if let Some(query) = url.query() {
        origin.push('?');
        origin.push_str(query);
    }

    let rewritten = rewrite_request(&header, method, &origin, &host);
    let target = TargetAddr::Domain(host.clone(), port);
    let mux_stream = session.open_stream(target).await?;
    mux_stream
        .send_data(Bytes::from(rewritten))
        .await
        .map_err(|_| BtProxyError::Protocol("send failed".to_string()))?;

    forward_body(stream, mux_stream).await?;
    Ok(())
}

fn parse_connect_target(path: &str) -> Result<(String, u16)> {
    let mut parts = path.split(':');
    let host = parts
        .next()
        .ok_or_else(|| BtProxyError::Protocol("missing host".to_string()))?
        .to_string();
    let port = parts
        .next()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(443);
    Ok((host, port))
}

fn rewrite_request(original: &[u8], method: &str, origin: &str, host: &str) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(format!("{} {} HTTP/1.1\r\n", method, origin).as_bytes());
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut req = Request::new(&mut headers);
    let _ = req.parse(original);
    let mut has_host = false;
    for header in req.headers.iter() {
        let name = header.name;
        if name.eq_ignore_ascii_case("Proxy-Connection") {
            continue;
        }
        if name.eq_ignore_ascii_case("Connection") {
            continue;
        }
        if name.eq_ignore_ascii_case("Host") {
            has_host = true;
        }
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(header.value);
        out.extend_from_slice(b"\r\n");
    }
    if !has_host {
        out.extend_from_slice(format!("Host: {}\r\n", host).as_bytes());
    }
    out.extend_from_slice(b"Connection: close\r\n\r\n");
    out
}

async fn tunnel(client: TcpStream, mux_stream: MuxStream) -> Result<()> {
    let (mut client_read, mut client_write) = client.into_split();
    let inbound = mux_stream.clone();

    let client_to_mux = tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            let n = client_read.read(&mut buf).await?;
            if n == 0 {
                let _ = mux_stream.send_fin().await;
                return Ok::<(), BtProxyError>(());
            }
            mux_stream
                .send_data(Bytes::copy_from_slice(&buf[..n]))
                .await
                .map_err(|_| BtProxyError::Protocol("mux send failed".to_string()))?;
        }
    });

    let mux_to_client = tokio::spawn(async move {
        while let Some(chunk) = inbound.recv_data().await {
            client_write.write_all(&chunk).await?;
        }
        let _ = client_write.shutdown().await;
        Ok::<(), BtProxyError>(())
    });

    let (res_client, res_mux) = tokio::try_join!(client_to_mux, mux_to_client)
        .map_err(|err| BtProxyError::Protocol(err.to_string()))?;
    res_client?;
    res_mux?;
    Ok(())
}

async fn forward_body(client: TcpStream, mux_stream: MuxStream) -> Result<()> {
    let (mut client_read, mut client_write) = client.into_split();
    let inbound = mux_stream.clone();

    let client_to_mux = tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            let n = client_read.read(&mut buf).await?;
            if n == 0 {
                let _ = mux_stream.send_fin().await;
                return Ok::<(), BtProxyError>(());
            }
            mux_stream
                .send_data(Bytes::copy_from_slice(&buf[..n]))
                .await
                .map_err(|_| BtProxyError::Protocol("mux send failed".to_string()))?;
        }
    });

    let mux_to_client = tokio::spawn(async move {
        while let Some(chunk) = inbound.recv_data().await {
            client_write.write_all(&chunk).await?;
        }
        let _ = client_write.shutdown().await;
        Ok::<(), BtProxyError>(())
    });

    let (res_client, res_mux) = tokio::try_join!(client_to_mux, mux_to_client)
        .map_err(|err| BtProxyError::Protocol(err.to_string()))?;
    res_client?;
    res_mux?;
    Ok(())
}
