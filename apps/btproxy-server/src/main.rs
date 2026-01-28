use anyhow::Result;
use btlink::BtLinkConfig;
use bytes::Bytes;
use clap::Parser;
use common::{init_tracing, ServerConfig};
use mux::{MuxConfig, MuxSession, Role, TargetAddr};
use socks5::connect_via_socks5;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = ServerConfig::parse();
    init_tracing(&cfg.log);

    let link_cfg = BtLinkConfig::default();
    #[cfg(target_os = "linux")]
    let link = btlink::accept_linux_rfcomm(cfg.channel, link_cfg).await?;

    #[cfg(target_os = "windows")]
    let link = btlink::accept_windows_rfcomm(cfg.channel, link_cfg).await?;

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    let link = {
        return Err(anyhow::anyhow!("unsupported platform"));
    };

    let mux_cfg = MuxConfig {
        max_frame: 65536,
        keepalive_ms: 10_000,
        psk: cfg.psk.as_ref().map(|s| s.as_bytes().to_vec()),
    };
    let session = MuxSession::start(link, mux_cfg, Role::Server).await?;

    info!("server ready");
    loop {
        if let Some((target, stream)) = session.accept_stream().await {
            let session = session.clone();
            let cfg = cfg.clone();
            tokio::spawn(async move {
                if let Err(err) = handle_stream(session, cfg, target, stream).await {
                    warn!(?err, "stream error");
                }
            });
        }
    }
}

async fn handle_stream(
    session: MuxSession,
    cfg: ServerConfig,
    target: TargetAddr,
    mux_stream: mux::MuxStream,
) -> Result<()> {
    let (host, port) = target_to_host_port(&target)?;
    let outbound = if cfg.direct {
        TcpStream::connect(format!("{}:{}", host, port)).await?
    } else {
        connect_via_socks5(
            &cfg.clash_socks,
            cfg.clash_user.as_deref(),
            cfg.clash_pass.as_deref(),
            &host,
            port,
        )
        .await?
    };

    session.send_open_ok(mux_stream.stream_id).await.ok();
    proxy_streams(outbound, mux_stream).await?;
    Ok(())
}

fn target_to_host_port(target: &TargetAddr) -> Result<(String, u16)> {
    match target {
        TargetAddr::Domain(host, port) => Ok((host.clone(), *port)),
        TargetAddr::IpV4(addr, port) => Ok((std::net::Ipv4Addr::from(*addr).to_string(), *port)),
        TargetAddr::IpV6(addr, port) => Ok((std::net::Ipv6Addr::from(*addr).to_string(), *port)),
    }
}

async fn proxy_streams(outbound: TcpStream, mux_stream: mux::MuxStream) -> Result<()> {
    let (mut outbound_read, mut outbound_write) = outbound.into_split();
    let inbound = mux_stream.clone();

    let mux_to_outbound = tokio::spawn(async move {
        while let Some(chunk) = inbound.recv_data().await {
            outbound_write.write_all(&chunk).await?;
        }
        Ok::<(), anyhow::Error>(())
    });

    let outbound_to_mux = tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            let n = outbound_read.read(&mut buf).await?;
            if n == 0 {
                let _ = mux_stream.send_fin().await;
                break;
            }
            mux_stream
                .send_data(Bytes::copy_from_slice(&buf[..n]))
                .await
                .map_err(|_| anyhow::anyhow!("mux send failed"))?;
        }
        Ok::<(), anyhow::Error>(())
    });

    let _ = tokio::try_join!(mux_to_outbound, outbound_to_mux)?;
    Ok(())
}
