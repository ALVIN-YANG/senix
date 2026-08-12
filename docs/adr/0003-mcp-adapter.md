# ADR-0003: Stateless, capability-shaped MCP Adapter

- Status: accepted
- Date: 2026-08-11

## Context

AI 客户端需要通过 MCP 查询和控制 Senix，但不能因此获得 Shell、数据库或比其 Credential 更高的权限。MCP 是另一个管理入口，不应复制 REST 的授权和流量规则。

当前 MCP `2026-07-28` 规范已经移除协议级会话。官方 Rust SDK `rmcp 3.1.2` 支持当前无会话 Streamable HTTP，也兼容 `2025-11-25` 客户端。

## Decision

新增独立的 `senix-mcp` Adapter，使用 `rmcp 3.1.2` 的 Streamable HTTP Service，并关闭旧协议的会话模式。每个 POST 是独立请求，共享状态只存在于 Senix 的领域 Module 和 SQLite 中。

MCP 挂载在管理端口的 `/mcp`。Bearer Key 由与 REST 相同的 HTTP 中间件逐请求认证；得到的 Principal 通过请求 Extension 传给 MCP 工具。工具只调用 `SecurityController`、`TrafficController`、`ConfigEngine` 和 `DiagnosticEngine`，不反向调用 REST，也不自行解释权限。

`tools/list` 是 Capability-shaped Tool Catalog。它按当前 Principal 的 Management Action 和可见 Instance 裁剪工具，并在新协议中标记为 private、零 TTL 缓存。这个目录用于减少 AI 误调用，不构成授权边界；每次 `tools/call` 仍重新执行领域授权，越权结果使用与 REST 相同的稳定错误码和 evidence。

首版工具不包含 Credential 管理、任意配置写入、Shell、SSH、Docker 或 Kubernetes。`plan_change` 只预检；在人工批准链路完成前，不暴露配置 apply/rollback。

MCP 默认只允许 loopback Host。私网或反向代理域名必须通过启动参数显式加入 Host 允许列表；浏览器来源必须加入 Origin 允许列表。管理面 TLS 仍由后续内置能力或受信反向代理解决。

## Consequences

- REST 与 MCP 共享相同 Key、Scope、领域操作和审计事实。
- MCP Adapter 无协议会话，重启和以后横向扩展不依赖粘性会话。
- 受限 Key 只看到与其任务相关的工具，减少 AI 上下文和无意义失败。
- 本机 stdio 桥尚未实现；它以后必须桥接同一控制面，不能建立另一套权限。

## Verification

系统测试通过真实 `/mcp` HTTP 请求验证：无 Key 返回 401；合法 Key 可以初始化；工具目录按权限裁剪；允许的实例读取和摘流成功；对范围外 Instance 的伪造调用仍返回 `FORBIDDEN`；摘流结果立即反映在 Pingora 数据面。

## References

- [MCP 2026-07-28 Streamable HTTP](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/specification/2026-07-28/basic/transports/streamable-http.mdx)
- [Official Rust SDK rmcp](https://github.com/modelcontextprotocol/rust-sdk)
