use clap::Parser;

#[derive(Debug, Clone, Parser)]
#[command(author, version, about)]
pub struct ClientConfig {
    #[arg(long, default_value = "127.0.0.1:18080")]
    pub listen: String,
    #[arg(long)]
    pub bt_addr: String,
    #[arg(long)]
    pub uuid: Option<String>,
    #[arg(long)]
    pub channel: Option<u8>,
    #[arg(long)]
    pub psk: Option<String>,
    #[arg(long, default_value = "info")]
    pub log: String,
}

#[derive(Debug, Clone, Parser)]
#[command(author, version, about)]
pub struct ServerConfig {
    #[arg(long, default_value = "22")]
    pub channel: u8,
    #[arg(long, default_value = "127.0.0.1:7891")]
    pub clash_socks: String,
    #[arg(long)]
    pub clash_user: Option<String>,
    #[arg(long)]
    pub clash_pass: Option<String>,
    #[arg(long, default_value = "false")]
    pub direct: bool,
    #[arg(long)]
    pub psk: Option<String>,
    #[arg(long, default_value = "info")]
    pub log: String,
}
