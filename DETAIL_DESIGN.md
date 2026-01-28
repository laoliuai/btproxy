# btproxy Rust 仓库骨架 + 关键文件清单 + 模块 TODO（给 Codex 直接开工）

> 目标：一键生成可编译的 workspace 骨架，并按 TODO 分解逐步补齐功能。  
> MVP：Windows/Ubuntu client + Windows/Ubuntu server；RFCOMM 单链路 + 自定义 MUX；A 本地 HTTP 代理（CONNECT + http）；B 侧可配置为 SOCKS5 出口到 Clash 或 direct。  
> 默认假设配对后的 client/server 彼此可信（两端对彼此拥有完全访问权限），PSK 仅作为可选增强。

---

## 0. 仓库结构（Workspace）

```
btproxy/
  Cargo.toml
  README.md
  DESIGN.md
  docs/
    repo_skeleton_todos.md   <-- 本文档（可选）
  crates/
    common/
      Cargo.toml
      src/
        lib.rs
        config.rs
        error.rs
        logging.rs
        net.rs
    btlink/
      Cargo.toml
      src/
        lib.rs
        link.rs
        framed_io.rs
        windows/
          mod.rs
          rfcomm.rs
        linux/
          mod.rs
          rfcomm.rs
    mux/
      Cargo.toml
      src/
        lib.rs
        frame.rs
        codec.rs
        handshake.rs
        session.rs
        stream.rs
        keepalive.rs
        protocol.md  <--（可选，写协议细节，便于对照）
    socks5/
      Cargo.toml
      src/
        lib.rs
        client.rs
        protocol.rs
    proxy_http/
      Cargo.toml
      src/
        lib.rs
        server.rs
        http1.rs
        connect.rs
        rewrite.rs
    common/
      Cargo.toml
      src/
        lib.rs
  apps/
    btproxy-client/
      Cargo.toml
      src/
        main.rs
    btproxy-server/
      Cargo.toml
      src/
        main.rs
```

---

## 1. 顶层文件清单（关键内容）

### 1.1 `Cargo.toml`（workspace）
- 定义 workspace members：`crates/*` 与 `apps/*`
- 统一依赖版本（可选：workspace.dependencies）

**TODO**
-  确定 MSRV（建议 Rust 1.76+）
-  开启 `resolver = "2"`

### 1.2 `README.md`（用户使用说明）
内容建议：
- 项目简介（显式 HTTP 代理 + RFCOMM + Clash）
- 快速开始（Windows/Ubuntu client / Windows/Ubuntu server）
- WSL 使用方式（设置 http_proxy/https_proxy）
- 常见问题（断连、重连、端口占用）

**TODO**
-  给出示例命令行
-  给出 Clash 端口配置示例（socks-port 7891）

### 1.3 `DESIGN.md`
放你已经确认的设计文档（上一份 markdown）。

---

## 2. Crate 级别拆分与职责

### 2.1 `crates/common`
**职责**：统一配置结构、错误类型、日志、通用网络工具

**文件**
- `config.rs`：CLI config struct（可被 apps 复用）/ 解析辅助
- `error.rs`：`thiserror` 定义统一错误
- `logging.rs`：tracing 初始化
- `net.rs`：小工具（读 header、限流 buffer、deadline 等）

**TODO**
-  定义统一 `BtProxyError`（分类：IO/Protocol/Auth/Timeout/Config）
-  提供 `init_tracing(level: &str)`（支持 `RUST_LOG`/CLI）
-  提供 `read_until_double_crlf(stream, max)`（HTTP header 读取）
-  提供 `Backoff`（指数退避 + jitter）

---

### 2.2 `crates/btlink`
**职责**：把 RFCOMM 连接抽象成“可靠字节流”，并提供“线程桥接 + 有界队列”的 async 友好接口。

**公共 API（建议）**
```rust
pub struct BtLink {
  pub tx: tokio::sync::mpsc::Sender<bytes::Bytes>,
  pub rx: tokio::sync::mpsc::Receiver<bytes::Bytes>,
}

pub enum BtRole { Client, Server }

pub struct BtLinkConfig {
  pub max_chunk: usize,         // e.g. 4096
  pub queue_bound: usize,       // e.g. 256
}

pub async fn connect_windows_rfcomm(
  addr: &str,
  uuid: Option<&str>,
  channel: Option<u8>,
  cfg: BtLinkConfig
) -> Result<BtLink>;

pub async fn accept_windows_rfcomm(channel: u8, cfg: BtLinkConfig) -> Result<BtLink>;

pub async fn accept_linux_rfcomm(channel: u8, cfg: BtLinkConfig) -> Result<BtLink>;
```

**文件**
- `link.rs`：BtLink 类型定义、spawn reader/writer 线程
- `framed_io.rs`：对 RFCOMM socket 的阻塞读写封装（read_exact / write_all）
- `windows/rfcomm.rs`：Winsock AF_BTH/BTHPROTO_RFCOMM connect
- `linux/rfcomm.rs`：BlueZ AF_BLUETOOTH/BTPROTO_RFCOMM bind/listen/accept

**TODO**
-  Windows：实现 `connect_windows_rfcomm`
  -  解析 BT_ADDR（`AA:BB:...`）为 64-bit
  -  若提供 UUID：通过系统 SDP（或 connect 结构）让其自动选 channel（能做到最好；做不到先要求 channel）
  -  支持 `--channel` fallback（直连）
  -  返回一个实现 `Read+Write` 的 socket handle，并交给 `BtLink::spawn(...)`
-  Windows：实现 `accept_windows_rfcomm`
  -  绑定/监听指定 channel（如系统限制则通过 SDP 注册 + 自动分配）
-  Linux：实现 `accept_linux_rfcomm`
  -  socket/bind/listen/accept
  -  支持只 accept 一个连接（MVP）
-  reader/writer 线程
  -  线程退出要向 async 层发一个“终止信号”（可用关闭 channel）
  -  有界队列满时 reader 阻塞，形成背压
-  统一关闭语义：任意一侧 EOF -> 关闭双方通道并回收资源

> 备注：macOS 适配先不写，但在 `btlink` 预留 `#[cfg(target_os="macos")]` 的模块入口。

---

### 2.3 `crates/mux`
**职责**：实现 BTPX MUX v1（帧编码/解码、握手、session、stream、多路复用、keepalive）

**公共 API（建议）**
```rust
pub struct MuxConfig {
  pub max_frame: usize,         // e.g. 65536
  pub keepalive_ms: u32,        // e.g. 10000
  pub psk: Option<Vec<u8>>,
}

pub struct MuxSession {
  // ...
}

impl MuxSession {
  pub async fn start(link: btlink::BtLink, cfg: MuxConfig, role: Role) -> Result<Self>;
  pub async fn open_stream(&self, target: TargetAddr) -> Result<MuxStream>;
  pub async fn shutdown(&self);
}

pub struct MuxStream {
  pub id: u32,
  // implements AsyncRead/AsyncWrite
}
```

**文件**
- `frame.rs`：Frame struct + enum（HELLO/OPEN/DATA/FIN/RST…）
- `codec.rs`：长度前缀拼帧（增量 decode）、encode
- `handshake.rs`：HELLO/HELLO_ACK、可选 PSK HMAC
- `session.rs`：session 状态机、stream map、dispatcher
- `stream.rs`：每个 stream 的 inbox/outbox、AsyncRead/AsyncWrite 适配
- `keepalive.rs`：PING/PONG、超时判定

**TODO**
-  定义帧类型
  -  `FrameType` 常量
  -  payload 结构化（推荐手写 encode/decode，避免 serde 依赖）
-  `FrameCodec`
  -  `push_bytes(&mut self, Bytes)` -> 产出 0..n 个 Frame
  -  max_frame 校验（超限断开）
-  握手
  -  role=client：发 HELLO，等 HELLO_ACK
  -  role=server：收 HELLO，回 HELLO_ACK
  -  可选 PSK：HMAC-SHA256 校验
-  Session runtime（核心）
  -  `decoder_task`：从 BtLink.rx 读取 bytes，拼帧，分发
  -  `encoder_task`：从全局 outgoing queue 取 Frame，编码写入 BtLink.tx
  -  `streams: HashMap<u32, StreamState>`
  -  OPEN 流程：
    -  client：send OPEN，await OPEN_OK/OPEN_ERR（带超时）
    -  server：收到 OPEN -> 调用上层 hook 建立 outbound -> 回 OK/ERR
-  Stream I/O
  -  stream inbox：mpsc<Bytes>（DATA）
  -  AsyncRead：从 inbox 读
  -  AsyncWrite：写 -> 发 DATA frame
  -  FIN/RST 处理：关闭本地读写并回收
-  Keepalive
  -  定时 PING
  -  收到 PING 回 PONG
  -  超时（3 个周期）触发 session 断开并通知上层

> 关键设计点：server 侧在 `session` 里应暴露一个 `on_open` 回调/trait，让 `btproxy-server` 注入“如何建立到 Clash 的 outbound”。

---

### 2.4 `crates/socks5`
**职责**：最小可用 SOCKS5 client（用于 B 端连接 Clash）

**文件**
- `protocol.rs`：常量与结构（greeting、auth、request/reply）
- `client.rs`：`connect_via_socks5(proxy_addr, target_addr, auth)` 返回 `TcpStream`

**TODO**
-  支持 methods：
  -  no-auth（0x00）
  -  user/pass（0x02，RFC1929）
-  支持 ATYP：
  -  domain
  -  ipv4/ipv6
-  错误处理：
  -  认证失败
  -  CONNECT reply 非 success
  -  超时（用 tokio timeout 包裹）
-  提供 `Socks5Config { proxy: SocketAddr, auth: Option<(String,String)> }`

---

### 2.5 `crates/proxy_http`
**职责**：A 端本地 HTTP/1.1 代理（CONNECT + http 绝对 URI -> origin-form rewrite）

**文件**
- `server.rs`：监听 accept，新连接 spawn handler
- `http1.rs`：header 读取与解析（基于 `httparse`）
- `connect.rs`：CONNECT 处理（建立 MuxStream + 200 + copy)
- `rewrite.rs`：普通 HTTP 请求 rewrite 并转发

**TODO**
-  监听 `127.0.0.1:18080`（默认）
-  每连接处理：
  -  读 header（`\r\n\r\n`，max 32KB）
  -  `httparse` 解析 method/path/version/headers
-  CONNECT：
  -  解析 `host:port`
  -  `mux.open_stream(target)`（带超时）
  -  成功写 `200 Connection Established`
  -  `copy_bidirectional`（本地 socket <-> MuxStream）
-  HTTP（绝对 URI）：
  -  `url::Url` parse path
  -  取 host/port/path+query
  -  open_stream(host, port_or_80)
  -  rewrite request line to origin-form
  -  rewrite headers（删 Proxy-Connection，强制 Connection: close，补 Host）
  -  转发 header + body（body 可直接把剩余缓冲 + 后续 read 流式发送）
  -  从 MuxStream 读响应转发给客户端，直到 EOF
-  简化策略：
  -  v0.1 不支持代理端 keep-alive，多请求复用（收到完响应直接关）
-  观测性：
  -  每个请求生成 request_id / stream_id 日志

---

## 3. Apps：入口程序（可直接跑）

### 3.1 `apps/btproxy-client`（Windows）
**职责**：解析 CLI → 初始化日志 → 启动 MuxSession（连接 RFCOMM）→ 启动本地 HTTP 代理

**main.rs TODO**
-  CLI（clap）参数：
  -  `--listen 127.0.0.1:18080`
  -  `--bt-addr`
  -  `--uuid`（默认 SPP UUID）
  -  `--channel`（可选）
  -  `--psk`（可选）
  -  `--log`
-  init tracing
-  连接管理：
  -  后台任务：循环 connect RFCOMM + handshake，生成 `MuxSessionHandle`
  -  session 掉线：触发代理层返回 502 或断开现有连接
  -  重连 backoff（common::Backoff）
-  启动 HTTP proxy：
  -  给 proxy_http 注入一个 `MuxProvider`（能拿到当前可用 session）
  -  open_stream 时若无 session：短等 2s，失败返回 502

> 实现建议：用 `ArcSwap` 或 `tokio::sync::watch` 保存当前 session（Some/None）。

---

### 3.2 `apps/btproxy-server`（Windows/Ubuntu）
**职责**：解析 CLI → init logging → accept RFCOMM → MuxSession(server role) → on_open 建立 outbound（Clash SOCKS 或 direct）

**main.rs TODO**
-  CLI（clap）参数：
  -  `--channel 22`
  -  `--clash-socks 127.0.0.1:7891`
  -  `--clash-user/--clash-pass`（可选）
  -  `--psk`（可选）
  -  `--sdp-external`（可选，先打印提示/或执行外部命令）
  -  `--log`
-  init tracing
  -  RFCOMM accept：
  -  accept 一个连接 -> start MuxSession(role=server)
  -  on_open hook：
  -  收到 OPEN(host,port)：
    -  根据配置：`socks5::connect(proxy=clash_socks, target=host:port, auth=...)` 或 direct TCP connect
    -  成功 -> OPEN_OK 并启动 stream<->tcp 双向 copy
    -  失败 -> OPEN_ERR 并关闭 stream
-  单 client 策略：
  -  新连接到来：默认拒绝（或踢旧连接，选一种写死）

---

## 4. 关键“胶水”接口：Mux 的 on_open 回调（Server 注入）

为避免 mux crate 直接依赖 socks5 / tokio::net，建议：

### 4.1 方案 A：Trait 回调（推荐）
在 `mux` 中定义 trait：

```rust
#[async_trait::async_trait]
pub trait OpenHandler: Send + Sync + 'static {
  async fn handle_open(&self, stream: MuxStream, target: TargetAddr) -> Result<()>;
}
```

- `MuxSession::start_server(link, cfg, handler: Arc<dyn OpenHandler>)`
- session 收到 OPEN：先创建 `MuxStream`，回 OPEN_OK/ERR 的逻辑由 session 控制；或由 handler 返回 Ok/Err 决定。

**TODO**
-  定义 `TargetAddr`（domain/ip + port）
-  规定 `handle_open` 返回 Err 时 session 发送 OPEN_ERR 并关闭 stream

### 4.2 方案 B：channel 事件（备选）
`MuxSession` 输出 `mpsc::Receiver<OpenEvent>` 给上层，上层处理后调用 `session.accept(stream_id)`/`session.reject(stream_id)`。

---

## 5. 开发模式（强烈建议）：TCP transport 替代 RFCOMM

为便于 CI/本地调试，建议在 btlink 增加一个 TCP 实现：

- `btlink::connect_tcp(addr)` / `btlink::accept_tcp(addr)`
- 用完全相同的 BtLink 线程桥接方式

**TODO**
-  `--transport rfcomm|tcp`
-  tcp 模式下：client 连接 server 的 tcp 端口（本机即可）
-  先把 mux + proxy + socks5 跑通，再接入 rfcomm

---

## 6. 最小可运行里程碑（按顺序实现）

### Milestone 1：MUX 编解码自测（无网络）
-  mux::FrameCodec 支持拼帧
-  单元测试：随机分片输入，输出帧正确

### Milestone 2：TCP transport 端到端（无蓝牙）
-  btlink tcp 实现
-  server：收到 OPEN -> 直接连目标 host:port（先不走 clash）做 echo 测试
-  client：本地 proxy CONNECT -> MUX -> server -> target
-  curl 验证 CONNECT

### Milestone 3：B 端接 Clash SOCKS5
-  socks5 client 完成
-  server on_open 走 socks5 连接 127.0.0.1:7891
-  访问 https 网站验证

### Milestone 4：替换为 RFCOMM（Windows + Ubuntu）
-  linux rfcomm accept
-  windows rfcomm connect（先要求用户提供 channel）
-  windows rfcomm accept（server 端）
-  跑通 CONNECT
-  再补 SDP/UUID 自动发现（若可行）

### Milestone 5：可用性增强
-  keepalive + 超时
-  重连 backoff
-  结构化日志（stream_id, target, bytes）

---

## 7. “先写死”的默认值（避免讨论不清）

- client listen：`127.0.0.1:18080`
- mux max_frame：65536
- btlink max_chunk：4096
- btlink queue_bound：256
- keepalive：10s，超时 30s
- open_stream 超时：10s
- proxy header max：32KB
- server channel：22（可配置）
- clash socks：`127.0.0.1:7891`

---

## 8. 代码风格与约束（Codex 指南）

- 所有网络 read/write 必须处理 partial read/write
- 所有跨任务共享结构用 `Arc<...>`，避免死锁
- stream 生命周期必须在一个地方集中回收（session 统一管理）
- `FIN`/`RST` 语义：v0.1 允许简化为“任意关闭即释放整个 stream”
- 对外错误：A 端 proxy 对用户返回 `502`/`504`，不要泄露内部细节过多
- 日志：用 `tracing`，每个 stream 带 `stream_id`、`target` field

---

## 9. 关键文件的“骨架内容建议”（每个至少要能编译）

> 建议 Codex 先把每个 crate 的 `lib.rs` 写成可编译的空实现 + TODO，再逐步填充。

### 9.1 `crates/mux/src/lib.rs`
- re-export 关键类型：`MuxSession, MuxStream, MuxConfig, TargetAddr`

### 9.2 `crates/btlink/src/lib.rs`
- `pub mod windows; pub mod linux;`（cfg）
- `pub use link::{BtLink, BtLinkConfig};`

### 9.3 `apps/*/main.rs`
- 最小：解析 args、init tracing、打印“not implemented”也要能跑
- 然后逐步接入 session + proxy

---

## 10. TODO 总览（可复制到 GitHub Issues）

### common
-  Backoff + jitter
-  read_http_header utility

### btlink
-  linux rfcomm accept
-  windows rfcomm connect (channel required first)
-  windows rfcomm accept (server)
-  tcp transport (dev mode)

### mux
-  frame definitions
-  codec (incremental decode)
-  handshake (optional PSK)
-  session runtime (dispatcher, stream map)
-  stream AsyncRead/Write
-  keepalive

### socks5
-  no-auth + user/pass
-  domain/ipv4/ipv6 connect
-  errors/timeouts

### proxy_http
-  CONNECT handling
-  http absolute-uri rewrite & forward
-  enforce Connection: close
-  map each connection to one mux stream

### apps
-  client reconnect loop + session provider
-  server on_open -> socks5/direct -> bidirectional copy
-  docs & examples
