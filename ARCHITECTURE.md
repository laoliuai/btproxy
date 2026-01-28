# btproxy 设计文档（RFCOMM 蓝牙隧道 + 本地显式 HTTP 代理 + Clash 出口）

版本：v0.1
目标平台：
- **Client**：Windows（宿主机运行；WSL 通过 Windows 的本地代理使用）或 Ubuntu
- **Server**：Windows 或 Ubuntu（Linux 使用 BlueZ；Windows 使用系统蓝牙栈）
- 未来：macOS（预留接口，不在 v0.1 强制实现）

---

## 0. 背景与约束

目标流程，下面称A为client，B为server
- A 上跑一个本地 **显式 HTTP 代理**（用户手动配置浏览器代理、或设置 `http_proxy/https_proxy`）
- A 通过 **蓝牙 Classic RFCOMM** 与 B 建立可靠字节流链路（不使用 PAN、不装虚拟网卡/驱动）
- A 把代理连接“隧道化”通过 RFCOMM 发给 B
- B 再把目标连接通过 **Clash** 的本地入站（HTTP 7890 / SOCKS 7891 / mixed-port）发出去，或者直接发出去
- 必须支持 HTTPS：采用 **CONNECT 隧道**（不解密）

关键点：RFCOMM 物理链路数量有限，现代浏览器会并发大量连接，因此必须做 **多路复用（MUX）**，在单条 RFCOMM 上承载多条逻辑子连接。

---

## 1. 目标与非目标

### 1.1 目标（MVP）
1. client 提供本地 HTTP 代理，支持：
   - `CONNECT host:port`（HTTPS 隧道）
   - 普通 `http://...` 代理请求（绝对 URI 请求行）
2. A↔B 使用 RFCOMM 单连接承载多路逻辑流（每个代理连接对应一个逻辑 stream）
3. B 对每个 stream，根据配置通过 Clash 的 SOCKS5（推荐 7891）或 direct 建立到目标 `host:port` 的 TCP 连接
4. 双向转发 DATA、支持并发、具备背压、可重连
5. WSL 可用：WSL 设置代理指向 Windows 本地端口即可

### 1.2 非目标（v0.1 不做）
- 透明代理（无需配置就接管所有流量）
- UDP/QUIC/HTTP3
- TLS 解密或 MITM
- macOS RFCOMM（先预留接口）
- 完整 HTTP keep-alive 复用（v0.1 可强制 `Connection: close` 简化）

---

## 2. 总体架构与数据流

### 2.1 模块图

(A client) (B server)
+------------------+ RFCOMM +---------------------+
| Local HTTP Proxy | <----------------> | RFCOMM Server |
| - CONNECT | | + MUX demux |
| - HTTP proxy | | + Stream manager |
+--------+---------+ +----------+----------+
| |
| (logical streams over MUX) | per-stream
v v
+----+-----------------+ +------+--------------+
| MUX (framing/streams)| | Clash or Direct Outbound |
+----+-----------------+ | (SOCKS5 to 7891, or direct) |
| +---------------------+
v
+-----+------+
| RFCOMM link |
+------------+


### 2.2 数据流（CONNECT）
1. 浏览器 → client 本地代理：`CONNECT example.com:443 HTTP/1.1`
2. client 创建 MUX stream，向 B 发送 `OPEN(stream_id, host, port)`
3. server 通过 Clash SOCKS5 CONNECT or direct CONNECT 到 `example.com:443`
4. 成功：server 回 `OPEN_OK(stream_id)`，随后双方在该 stream 上双向转发 `DATA`
5. 任意一侧关闭：发 `FIN`/`RST`，释放资源

### 2.3 数据流（普通 HTTP 代理请求）
浏览器访问 `http://example.com/` 时可能发：

`GET http://example.com/path HTTP/1.1`

client 解析 URL 得到 `host=example.com port=80 path=/path`，建立 stream 后，将请求行改写为 origin-form：

`GET /path HTTP/1.1`

并强制 `Connection: close`（简化实现），把请求转发到目标 80 端口，收到响应后关闭连接。

---

## 3. 协议设计：BTPX MUX v1（自定义轻量多路复用）

> 目标：实现简单、跨平台、可背压、可扩展。只需可靠字节流（RFCOMM）。

### 3.1 连接级帧格式

所有消息都用“长度前缀帧”：
```
Frame := LEN(u32be) | TYPE(u8) | PAYLOAD(LEN-1 bytes)
```

- `LEN` 是后续 `TYPE+PAYLOAD` 的字节数（网络序大端）
- 单帧最大建议：64 KiB（可配置；超出直接断开，防止内存攻击）

### 3.2 帧类型与载荷

#### 0x01 HELLO（握手）
Payload：
- version(u16be) = 1
- flags(u16be)   = bit0: psk_enabled, bit1: gzip_reserved...
- max_frame(u32be)
- keepalive_ms(u32be)
- nonce(u64be)（随机）
- [optional] hmac(32 bytes)（若启用 PSK）

#### 0x02 HELLO_ACK
同 HELLO 结构，nonce 为对端 nonce 的回显或新 nonce（保持对称即可）

**PSK（可选）**：
- 双方配置同一个 `--psk`
- `hmac = HMAC-SHA256(psk, nonce||fixed_string||version)`
- 不匹配则断开
- 用途：防止同蓝牙环境下非本程序连接（即便已配对）

#### 0x10 OPEN（A→B，打开逻辑流）
Payload：
- stream_id(u32be)（client 单调递增）
- addr_type(u8) 1=domain 2=ipv4 3=ipv6
- host_len(u16be)（domain 时）
- host_bytes（domain 时）
- ip_bytes（ipv4=4 / ipv6=16）
- port(u16be)
- proto(u8) 1=tcp（预留）

#### 0x11 OPEN_OK（B→A）
Payload：
- stream_id(u32be)

#### 0x12 OPEN_ERR（B→A）
Payload：
- stream_id(u32be)
- code(u16be) 例如：1=clash_connect_fail 2=dns_fail 3=forbidden 4=timeout
- msg_len(u16be)
- msg_bytes

#### 0x20 DATA（双向）
Payload：
- stream_id(u32be)
- data_len(u16be)
- data_bytes

> 注：`LEN` 已含总长度，data_len 可省略；保留便于校验与调试。

#### 0x21 FIN（双向，半关闭/结束）
Payload：
- stream_id(u32be)

语义：发送方不会再发 DATA；接收方仍可继续发 DATA（类似 TCP half-close）。
v0.1 可简化为：收到 FIN 即关闭整个 stream。

#### 0x22 RST（双向，强制关闭）
Payload：
- stream_id(u32be)
- code(u16be)

#### 0x30 PING / 0x31 PONG（保活）
Payload：
- nonce(u64be)

策略：无数据时每 10s 发 PING；连续 3 个周期无响应则断开并重连。

---

## 4. client端详细设计

### 4.1 进程职责
- 提供本地 HTTP 代理监听（默认 `127.0.0.1:18080`）
- 维护到 server 的 RFCOMM 连接（自动重连）
- 在 RFCOMM 上运行 MUX，管理 stream 生命周期
- 将本地代理连接映射为远端 stream，并转发数据

### 4.2 HTTP 代理（MVP 行为）

#### 4.2.1 CONNECT
- 读到完整请求头（`\r\n\r\n` 结束）
- 解析：`CONNECT host:port HTTP/1.1`
- 调用 `mux.open_stream(host, port)` 得到 `MuxStream`
- 成功后回：`HTTP/1.1 200 Connection Established\r\n\r\n`
- 启动双向 copy：
  - client_socket → mux_stream.write(DATA)
  - mux_stream.read(DATA) → client_socket

#### 4.2.2 普通 HTTP 代理请求（绝对 URI）
- 请求行：`METHOD http://host[:port]/path?query HTTP/1.1`
- 提取 host/port/path
- `mux.open_stream(host, port_or_80)`
- 请求改写：
  - 请求行改为 origin-form：`METHOD /path?query HTTP/1.1`
  - 确保 `Host` 存在
  - 删除 `Proxy-Connection`
  - 强制 `Connection: close`
- 转发请求头 + body（流式）
- 转发响应直到远端关闭，然后关闭本地连接

> v0.1 强制 close 用于规避 keep-alive、多请求复用、复杂 body 编码边界。

### 4.3 MUX 会话管理
- client 端维护 `MuxSession`：
  - `session_state`: Connected/Connecting/Disconnected
  - `next_stream_id: u32` 从 1 递增
  - `streams: HashMap<u32, StreamHandle>`
- RFCOMM 断开：
  - 所有活跃 stream 本地关闭
  - open_stream 等待者返回错误
  - 后台重连

### 4.4 RFCOMM 连接（Windows）
CLI：
- `--bt-addr AA:BB:CC:DD:EE:FF`
- `--uuid <SPP_UUID>`（默认 SPP UUID）
- `--channel <n>`（可选：给定则不 SDP）

策略：
- 优先：按 UUID 让系统通过 SDP 自动解析 channel
- 失败：若提供 `--channel` 则直连该 channel

实现方式：
- Winsock socket：`AF_BTH / SOCK_STREAM / BTHPROTO_RFCOMM`
- connect 到 `SOCKADDR_BTH`
- 建连后交给 `BtLink` 作为字节流

---

## 5. server 端详细设计

### 5.1 进程职责
- 监听 RFCOMM（固定 channel，例如 22，可配置）
- 接入 MUX，接收 OPEN 并创建对应 outbound
- 对每个 stream，根据配置，通过 Clash（推荐 SOCKS5 7891） or direct 建立连接
- 双向转发 DATA
- 可选：注册 SDP 服务（方便 Windows 通过 UUID 自动发现 channel）

### 5.2 RFCOMM 监听（Windows or Ubuntu/BlueZ）
- Linux：`socket(AF_BLUETOOTH, SOCK_STREAM, BTPROTO_RFCOMM)` + bind `BDADDR_ANY` + channel + listen/accept
- Windows：Winsock `AF_BTH / SOCK_STREAM / BTHPROTO_RFCOMM` 绑定并监听指定 channel（如系统不支持绑定 channel，则采用 SDP 注册 + 由系统分配）

v0.1 仅支持单 client 连接；新连接策略：
- 默认拒绝新连接（默认行为，根据配置文件，也可踢掉旧连接切换到新连接，二选一）

### 5.3 SDP 注册（推荐但可选）
建议 server 注册 SPP 服务记录，便于 client 自动发现 channel：

- v0.1：提供 `--sdp-external`，启动时尝试调用外部工具注册（例如 `sdptool`）；失败则提示用户手动执行
- v0.2：实现 BlueZ D-Bus ProfileManager1 原生注册（更稳）

### 5.4 Clash 出口（B → Clash）
默认 SOCKS5：
- `--clash-socks 127.0.0.1:7891`
- 可选鉴权：`--clash-user user --clash-pass pass`

每个 stream 的 outbound（SOCKS5）：
1. TCP connect 到 Clash socks-port
2. SOCKS5 greeting：
   - methods: 0x00(no auth) 或 0x02(user/pass)
3. 若 user/pass：RFC1929 子协商
4. SOCKS5 CONNECT 到目标 host:port
5. 成功：回 OPEN_OK；失败：回 OPEN_ERR 并关闭 stream

---

或者直接根据配置文件，direct连接

## 6. 关键工程实现：跨平台 BtLink（RFCOMM 作为字节流）

建议采用“阻塞 IO + 专用线程 + 有界队列”的桥接方式，减少异步 socket 适配复杂度。

设计：
- `BtLinkReaderThread`：阻塞 read RFCOMM socket → push `Vec<u8>` 到有界队列
- `BtLinkWriterThread`：从有界队列 pop `Vec<u8>` → 阻塞 write 到 RFCOMM socket
- async 侧提供：
  - `send_bytes(Bytes)`：入 writer 队列
  - `recv_bytes() -> Bytes`：从 reader 队列取数据供 frame decoder 消费

背压由有界队列天然提供。
另外，需要考虑BtLink需要统一接口，同时支持windows/ubuntu不同底层实现

---

## 7. 任务与并发模型（Tokio）

### 7.1 连接级任务
- `bt_link_task`：维护 RFCOMM 连接、握手、重连
- `frame_decode_task`：从 `BtLink.recv_bytes` 拼帧、解析 frame、分发到 stream inbox
- `frame_encode_task`：从 outgoing 队列取 frame → 写入 BtLink
- `keepalive_task`：周期 PING/PONG，超时断开

### 7.2 Stream 级任务
- `local_to_remote`：本地 socket read → DATA frame → outgoing
- `remote_to_local`：stream inbox 收到 DATA → 写到本地 socket

释放：
- 任意方向 EOF/error：发送 FIN/RST，关闭另一侧并回收 stream

---

## 8. 错误处理与重连策略

### 8.1 Client端
- RFCOMM 断开：
  - `MuxSession` 标记 Disconnected
  - 所有 stream 失败并释放
  - 已建立 CONNECT 的连接将感知 EOF/错误（浏览器会重试）
- 重连 backoff：`1s,2s,4s,8s,16s` 上限 30s + jitter
- Disconnected 状态收到新代理连接：
  - 最多等待 2s 尝试连上，否则返回 `502 Bad Gateway`

### 8.2 Server 端
- Clash or direct 连接失败/鉴权失败：对该 stream 回 OPEN_ERR 并关闭
- RFCOMM 断开：清理所有 stream

---

## 9. 安全与默认策略

- client 的本地代理默认仅监听 `127.0.0.1`
- server 连接 Clash 默认仅连 `127.0.0.1`
- 默认假设 client/server 主机之间是可信的（两端对彼此拥有完全访问权限），无需额外鉴权
- 可选 `--psk`：避免非本程序连接（即便蓝牙已配对）
- v0.2 可加目的地限制（deny RFC1918 等）

---

## 10. CLI 设计（建议）

### 10.1 A（btproxy-client）
```
btproxy-client ^
--listen 127.0.0.1:18080 ^
--bt-addr AA:BB:CC:DD:EE:FF ^
--uuid 00001101-0000-1000-8000-00805F9B34FB ^
[--channel 22] ^
[--psk <hex-or-string>] ^
[--log info]
```

增强：
- `--list-devices`：列出已配对设备及地址（Windows API）
- `--name <device_name>`：按名字匹配地址

### 10.2 B（btproxy-server）
```
btproxy-server
--channel 22
--clash-socks 127.0.0.1:7891
[--clash-user xxx --clash-pass yyy]
[--psk ...]
[--sdp-external]
[--log info]
```


---

## 11. Rust 代码结构（workspace 建议）

```
btproxy/
  Cargo.toml (workspace)
  crates/
    btlink/ # RFCOMM link abstraction + per-OS impl
    mux/ # framing, session, stream, handshake
    proxy_http/ # HTTP proxy server (CONNECT + http)
    socks5/ # minimal SOCKS5 client (to Clash)
    common/ # config, errors, logging
  apps/
    btproxy-client/
      main.rs
    btproxy-server/
      main.rs
```


依赖建议：
- async & IO：`tokio`, `bytes`, `tokio-util`
- HTTP 解析：`httparse`, `url`
- CLI：`clap`
- 日志：`tracing`, `tracing-subscriber`
- 错误：`thiserror`, `anyhow`
- crypto（PSK 可选）：`hmac`, `sha2`
- Windows：`windows` 或 `windows-sys`
- Linux：`libc`, `nix`

---

## 12. 核心接口定义（给 Codex 开工）

### 12.1 MuxSession（示意）
```rust
struct MuxSession {
  // state, stream map, outgoing sender, etc.
}

impl MuxSession {
  async fn open_stream(&self, host: String, port: u16) -> Result<MuxStream>;
  async fn send_data(&self, stream_id: u32, data: Bytes) -> Result<()>;
  async fn close_stream(&self, stream_id: u32);
}
```

### 12.2 MuxStream（对上层表现为 AsyncRead/AsyncWrite）

+ 内部持有 stream_id

    + write() -> 发送 DATA frame

    + read() <- 从 stream inbox 取 DATA

### 12.3 BtLink（线程桥接）
```rust
struct BtLink {
  rx: mpsc::Receiver<Bytes>, // raw bytes from socket
  tx: mpsc::Sender<Bytes>,   // raw bytes to socket
}
```

## 13. HTTP 代理实现细节（固定行为减少歧义）
### 13.1 请求头读取与解析

+ 每个新 TCP 连接读取到 \r\n\r\n（限制最大 header size，例如 64KB）

+ httparse 解析

+ CONNECT：req.path 为 host:port

+ 普通 HTTP：req.path 为绝对 URI，用 url parse

### 13.2 Header 重写（普通 HTTP）

+ 删除：Proxy-Connection

+ 设置/覆盖：Connection: close

+ 若无 Host 则补

+ 其余原样转发

## 14. 兼容性与使用体验
### 14.1 配对与首次使用

+ 使用系统 UI/命令完成 Windows 与 Ubuntu 的蓝牙配对（一次），并优先走 SPP UUID 自动发现
+ 提供设备列表与按名称连接的入口（Windows：已配对设备枚举；Linux：可配合 bluetoothctl）

+ B 启动 server（固定 channel）

+ A 启动 client（指定 bt addr）

+ 浏览器设置代理为 127.0.0.1:18080

### 14.2 后续使用

+ B 常驻服务

+ A 每次启动自动连接/重连

### 14.3 WSL 使用

WSL 中：
```
export http_proxy=http://127.0.0.1:18080
export https_proxy=http://127.0.0.1:18080
```

若 127.0.0.1 不通，再改为 Windows 宿主机 IP。

## 15. 测试计划
### 15.1 单元测试

+ mux framing encode/decode（随机切片输入，确保正确拼帧）

+ OPEN/OPEN_ERR 状态机

+ socks5 handshake（本地 mock socks server）

### 15.2 集成测试（不依赖蓝牙）

+ 用 TCP loopback 代替 BtLink（开发模式 --transport tcp），验证：

    + CONNECT 可用

    + 并发多连接

    + 断链重连行为

### 15.3 手工测试（真实设备）

+ Windows 浏览器访问 https 网站

+ curl -x http://127.0.0.1:18080 https://example.com -v

+ WSL 内 curl 走代理

## 16. 迭代路线图

+ v0.1：Windows/Ubuntu client + Windows/Ubuntu server，固定 channel，Clash SOCKS/Direct 出口，HTTP proxy(CONNECT + http close)

+ v0.2：Ubuntu 原生 SDP 注册（BlueZ D-Bus），Windows 设备枚举/按名连接

+ v0.3：macOS 支持（IOBluetooth FFI）

+ v0.4：更完整的 HTTP keep-alive、连接池、流控优化

## 17. 交付物清单

1. btproxy-client（Windows/Ubuntu）

    + 本地 HTTP 代理

    + RFCOMM 连接 + MUX

    + WSL 使用说明

2. btproxy-server（Windows/Ubuntu）

    + RFCOMM 监听

    + MUX + per-stream outbound via SOCKS5 to Clash 或 direct

3. 文档：

    + 配对步骤与示例命令

    + Clash 配置（确认 socks-port/mixed-port 开启）

    + FAQ（断连、重连、端口占用）
