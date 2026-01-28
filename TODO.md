# 待开发项（按方便易用 → 性能/可靠性 → 安全优先级排序）

## 方便易用
- **完善用户文档与示例**：补齐 README 的快速开始、Clash socks-port 配置示例、WSL 代理设置示例、常见问题。参考设计文档 TODO 中的 README 要求。 
- **设备发现/按名连接（Windows）**：实现 `--list-devices` 与 `--name` 等便捷参数，按配对设备名解析地址。 
- **UUID/SDP 自动发现**：
  - Server 侧支持 SDP 注册（BlueZ D-Bus 或 `--sdp-external` 方式），让 client 可用 UUID 自动发现 channel。
  - Client 侧在 Windows/Linus 尝试 UUID 自动解析 channel，避免手动 `--channel`。 
- **开发模式（TCP transport）**：提供 `--transport tcp`，方便在无蓝牙环境下联调与 CI，设计文档里已建议。 
- **Server 端重连/多连接策略**：RFCOMM 断开后自动回到 accept 状态，并提供“拒绝新连接/踢旧连接”策略配置。 
- **更清晰的错误返回**：
  - Proxy 端在 session 不可用时返回 502/504（含超时）。
  - Server 端 outbound 连接失败时发送 `OPEN_ERR`（当前仅在成功时回 `OPEN_OK`）。
- **增强日志可观测性**：在 proxy/mux 日志中加入 `stream_id`、`target`、bytes 统计，便于排障与性能分析。 

## 性能 / 可靠性
- **Mux keepalive 超时与断线检测**：实现 PING/PONG 超时计数、触发 session 关闭与上层重连。 
- **open_stream 超时与取消**：为 open/handshake 增加超时与取消，避免挂起。 
- **FIN/RST/半关闭语义完善**：目前实现简化；按文档补充半关闭/错误码传递，统一回收资源。 
- **SOCKS5 功能完善**：支持 IPv4/IPv6 ATYP、超时控制、错误码映射；支持 direct 与 socks5 的更细粒度 fallback。 
- **HTTP 代理能力增强**：
  - v0.1 仍可坚持 `Connection: close`，但完善 body 读写、异常关闭处理。
  - 后续支持 keep-alive/多请求复用。 
- **压力与单元测试**：
  - mux 编解码随机分片测试。
  - socks5 握手/认证测试。
  - TCP transport 集成测试（CONNECT 走通）。 

## 安全
- **目的地访问控制**：增加可配置的目标地址限制（如拒绝 RFC1918/环回地址），降低滥用风险。 
- **PSK/鉴权体验增强**：
  - 统一 CLI 配置提示与错误信息。
  - 握手失败日志与连接拒绝原因更明确。 
- **资源限额与防护**：
  - 限制单连接并发 stream 数/单 stream 缓冲。
  - 限制最大 header / frame / 数据流速率，避免 DoS。 
