use anyhow::Result;
use btlink::BtLinkConfig;
use clap::Parser;
use common::{init_tracing, Backoff, ClientConfig};
use mux::{MuxConfig, MuxSession, Role};
use proxy_http::run_http_proxy;
use tokio::time::{sleep, Duration};
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = ClientConfig::parse();
    init_tracing(&cfg.log);

    let mut backoff = Backoff::new(1000, 30_000);
    loop {
        match connect_session(&cfg).await {
            Ok(session) => {
                backoff.reset(1000);
                if let Err(err) = run_http_proxy(&cfg.listen, session).await {
                    error!(?err, "proxy exited");
                }
            }
            Err(err) => {
                error!(?err, "failed to connect");
            }
        }

        let delay = backoff.next_delay();
        info!("reconnecting in {} ms", delay);
        sleep(Duration::from_millis(delay)).await;
    }
}

async fn connect_session(cfg: &ClientConfig) -> Result<MuxSession> {
    let link_cfg = BtLinkConfig::default();
    #[cfg(target_os = "linux")]
    let link = {
        let channel = cfg
            .channel
            .ok_or_else(|| anyhow::anyhow!("--channel required on linux"))?;
        btlink::connect_linux_rfcomm(&cfg.bt_addr, channel, link_cfg).await?
    };

    #[cfg(target_os = "windows")]
    let link = {
        btlink::connect_windows_rfcomm(&cfg.bt_addr, cfg.uuid.as_deref(), cfg.channel, link_cfg)
            .await?
    };

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    let link = {
        return Err(anyhow::anyhow!("unsupported platform"));
    };

    let mux_cfg = MuxConfig {
        max_frame: 65536,
        keepalive_ms: 10_000,
        psk: cfg.psk.as_ref().map(|s| s.as_bytes().to_vec()),
    };
    let session = MuxSession::start(link, mux_cfg, Role::Client).await?;
    Ok(session)
}
