# btproxy

Bluetooth RFCOMM tunnel with HTTP proxy for bypassing network restrictions.

## Overview

btproxy creates a secure tunnel between Windows client and Ubuntu server via Bluetooth Classic RFCOMM, routing through Clash proxy. It provides an explicit HTTP proxy that can be used by browsers or command-line tools.

### Architecture

```
(A Windows) (B Ubuntu)
+------------------+ RFCOMM +---------------------+
| Local HTTP Proxy | <----------------> | RFCOMM Server |
| - CONNECT | | + MUX demux |
| - HTTP proxy | | + Stream manager |
+--------+---------+ +----------+----------+
```

## Features

- **MVP Features:**
  - Local HTTP proxy (CONNECT + HTTP requests)
  - RFCOMM single connection with custom multiplexing
  - Clash SOCKS5 outbound integration
  - HTTPS tunnel support (CONNECT method)
  - WSL compatibility

- **Current Status:**
  - ✅ Architecture design complete
  - ✅ Detailed implementation plan ready
  - 🚧 Rust workspace skeleton created
  - 🚧 Core modules in development

## Quick Start

### Prerequisites

- Rust 1.76+ with Cargo
- Bluetooth support on both machines
- Clash proxy running on Ubuntu server

### Build & Deploy (构建与部署)

#### 1) Ubuntu Server (Linux) 依赖与服务

Install Bluetooth utilities and start the daemon:

```bash
sudo apt install -y bluez bluez-tools
sudo systemctl enable --now bluetooth
sudo rfkill unblock bluetooth
```

Get the server Bluetooth address (used by the Windows client as `--bt-addr`):

```bash
bluetoothctl show
```

#### 2) 蓝牙配对与信任 (Ubuntu ↔ Windows)

On Ubuntu (server), pair and trust the Windows client:

```bash
bluetoothctl
power on
agent on
default-agent
discoverable on
scan on            # find Windows address (AA:BB:CC:DD:EE:FF)
pair AA:BB:CC:DD:EE:FF
trust AA:BB:CC:DD:EE:FF
connect AA:BB:CC:DD:EE:FF
quit
```

On Windows, open **Settings → Bluetooth & devices**, add the Ubuntu device, and complete pairing.

#### 3) RFCOMM Channel (可选但推荐)

btproxy uses a fixed RFCOMM channel. Ensure both sides match the same `--channel` value (e.g. 22).
On Ubuntu, you can advertise the RFCOMM service channel so Windows can find it more easily:

```bash
sudo sdptool add --channel=22 SP
```

#### 4) 启动服务

Start btproxy-server on Ubuntu:

```bash
./target/release/btproxy-server \
    --channel 22 \
    --clash-socks 127.0.0.1:7891 \
    [--clash-user user] \
    [--clash-pass pass]
```

Start btproxy-client on Windows:

```bash
./target/release/btproxy-client \
    --listen 127.0.0.1:18080 \
    --bt-addr AA:BB:CC:DD:EE:FF \
    --uuid 00001101-0000-1000-8000-00805F9B34FB \
    [--channel 22]
```

### Building

```bash
cargo build --release
```

### Configure Browser/Client

Set your browser or client proxy to: `http://127.0.0.1:18080`

## Development

### Project Structure

```
btproxy/
├── Cargo.toml                    # Workspace configuration
├── ARCHITECTURE.md               # System architecture & data flow  
├── DETAIL_DESIGN.md              # Rust workspace + implementation plan
├── crates/
│   ├── common/                   # Shared types, config, errors, logging
│   ├── btlink/                   # RFCOMM abstraction + OS implementations
│   ├── mux/                      # Framing, session, stream management
│   ├── socks5/                   # SOCKS5 client for Clash integration
│   └── proxy_http/               # HTTP proxy server implementation
└── apps/
    ├── btproxy-client/           # Windows client application
    └── btproxy-server/           # Ubuntu server application
```

### Development Mode

For easier testing without Bluetooth, use TCP transport mode:

```bash
# Server
./target/release/btproxy-server --transport tcp --listen 127.0.0.1:18888

# Client  
./target/release/btproxy-client --transport tcp --server-addr 127.0.0.1:18888
```

## Testing

Run tests:

```bash
cargo test
```

Run specific crate tests:

```bash
cargo test -p mux
cargo test -p btlink
```

## Protocol

btproxy uses a custom multiplexing protocol (BTPX MUX v1) over RFCOMM:

- Frame format: `LEN(u32be) | TYPE(u8) | PAYLOAD(LEN-1 bytes)`
- Frame types: HELLO/OPEN/DATA/FIN/RST/PING/PONG
- Stream-based multiplexing for concurrent connections

## Security

- Optional PSK authentication to prevent unauthorized connections
- Local proxy binds to 127.0.0.1 by default
- Clash integration binds to 127.0.0.1 by default

## License

MIT
