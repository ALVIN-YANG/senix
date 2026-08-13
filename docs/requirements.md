# Senix 产品需求与交付边界

> 状态：v0.3 已实现边界
>
> 更新：2026-08-14
>
> 许可证：Apache-2.0

## 1. 产品定义

Senix 是一个基于 Rust 和 Pingora 的独立网关，不依赖 Nginx，也不是 Pingora 或 Pingap 的插件。它面向个人开发者和小团队，让一个人配合 AI 客户端完成网关配置、实例摘流、故障诊断和日常维护。

一个 `senixd` 进程承载数据面、控制面、REST、MCP 和嵌入式管理后台。数据面按不可变 Snapshot 处理请求，不查询 SQLite；控制面用本机 SQLite 持久化配置、Key 摘要、审计、证书密文和实例期望状态。

Senix 只管理网关和流量。SSH、Shell、容器更新、Kubernetes 操作、部署顺序和灰度判断由用户、CI 或外部脚本负责。

### 1.1 核心价值

1. 安全变更：配置先校验并生成不可修改的 Change Plan，由 Owner 批准精确内容后原子发布。
2. 部署流量控制：向脚本提供摘流、在途查询、回接、权重调整和禁用等幂等原语。
3. 可解释诊断：从数据面同一 Snapshot 输出路由、候选实例、健康与流量状态证据。
4. AI 安全入口：MCP 工具按 Key 权限和 Instance 范围裁剪，调用时再次鉴权并写审计。

### 1.2 目标用户

- 在一台或少量 Linux 服务器上运行多个 Web 服务的个人开发者。
- 使用 systemd、Docker Compose 或简单 CI 脚本的小团队。
- 希望让 AI 协助排障和流量操作，但不愿授予服务器 Shell 权限的用户。

## 2. v0.3 已交付能力

### 2.1 代理与协议

| 能力 | v0.3 边界 |
| --- | --- |
| 下游 HTTP | 明文 HTTP/1.1；TLS 监听支持 HTTP/1.1 和 HTTP/2 ALPN |
| 上游 HTTP | HTTP/1.1；HTTPS 可选择 HTTP/1.1、HTTP/2 或协商两者 |
| TLS | 下游静态 PEM 或托管 SNI 证书；上游系统 CA、SNI 和主机名校验 |
| WebSocket | HTTP/1.1 Upgrade 透明代理 |
| SSE / gRPC | 按流透明代理，并单独计入长连接在途数 |
| HTTP-01 | 优先响应当前 ACME Challenge，不进入普通路由 |

当前不承诺 h2c、HTTP/2 WebSocket、gRPC-Web 协议转换、下游或上游 mTLS、HTTP/3、QUIC、通用 TCP/UDP 代理。已有连接不能迁移到另一个后端或另一个网关进程。

### 2.2 路由与后端

- 精确域名和 `*.example.com` 形式的单标签通配域名。
- 边界安全的最长路径前缀匹配；`/api` 不会误匹配 `/apix`。
- 每条 Route 直接包含后端实例；实例有稳定 ID、地址、部署代次和权重。
- 平滑加权轮询，不把新请求分配给摘流、禁用或不健康实例。
- TCP 或 HTTP 主动健康检查，支持间隔、超时、连续成功和连续失败阈值。
- HTTPS 后端的 HTTP 健康检查复用同一证书校验、SNI 和 ALPN 设置。
- 配置拒绝未知字段、重复路由、无效 Host、重复或冲突 Instance、零总权重及不安全检查参数。

v0.3 尚无独立 Service 和 BackendPool 实体，也不支持按方法、请求头或正则匹配，以及重定向、改写和请求头修改动作。

### 2.3 配置变更

```text
candidate
  -> validate + structured route diff + content digest
  -> immutable Change Plan
  -> Owner approval, valid for 15 minutes
  -> apply against the same base Snapshot
  -> SQLite commit
  -> atomic data-plane publish
```

- 计划绑定基线版本和候选内容摘要，持久化后不能修改。
- 无效计划可以查看，但不能批准或应用。
- 基线变化、批准过期或内容摘要不一致时拒绝应用。
- 回滚先从历史 Snapshot 创建新计划，再走相同批准和应用流程。
- MCP 可以规划，也可以在有 `change.apply` 权限时应用已由 Owner 批准的计划；MCP 和 API Key 不能批准。
- 数据库为空时从 JSON 初始化第一份 Snapshot；已有 Snapshot 时忽略启动 JSON，避免重启覆盖现场状态。

### 2.4 实例流量控制

期望流量状态与健康状态分开保存：

- `SERVING`：按权重接收新请求。
- `DRAINING`：停止新请求，等待在途请求结束。
- `DRAINED`：普通请求已清零；长连接可能仍存在。
- `DISABLED`：保持隔离，直到收到明确回接。
- `UNKNOWN`、`HEALTHY`、`UNHEALTHY`：独立表示主动健康结果。

写操作包括 `drain`、`rejoin`、`set_weight` 和 `disable`，全部要求 `Idempotency-Key`。重复请求返回原结果。默认拒绝摘除最后一个可用后端，也拒绝回接不健康实例；只有显式 `force=true` 才能覆盖，并记录高风险审计。

摘流返回持久化 `operation_id`。到期只进入 `DRAIN_TIMEOUT`，不强杀或迁移连接。部署脚本根据普通和长连接在途数决定继续等待还是执行后续动作。

### 2.5 身份、权限和审计

- 服务器本机 CLI 创建一次性 Owner Credential，再从标准输入建立唯一 Owner 账号；账号建立后引导 Credential 在同一事务失效。
- Owner 密码使用 Argon2id；浏览器会话使用短期签名 Cookie、`HttpOnly`、`SameSite=Strict` 和同源 CSRF 检查。
- 登录失败按真实 TCP 来源限速，不信任 `X-Forwarded-For`；5 分钟内失败 5 次锁定 15 分钟，同时最多执行 2 次 Argon2 校验。
- API Key 完整值只展示一次，数据库只保存摘要；Key 有动作、Instance 范围、有效期和撤销状态。
- REST、管理后台和 MCP 共用 `SecurityController` 的认证、授权和审计规则。
- 审计记录操作者、动作、资源、结果和风险，不保存密码、Cookie、Key、Authorization、私钥或请求正文。
- 所有管理错误和登录响应禁止缓存。

管理端口默认绑定 `127.0.0.1:9080`。v0.3 不内置管理面 TLS；不要直接暴露到公网。通过受信 TLS 入口访问时启用 `--admin-secure-cookie`，并显式配置 MCP 允许的 Host 和 Origin。

### 2.6 MCP

MCP 使用无会话 Streamable HTTP，挂载在管理端口 `/mcp`。每个请求独立携带 Bearer Key，不接受浏览器 Cookie。

当前工具：

- `list_instances`、`get_instance_health`
- `drain_instance`、`get_drain_status`、`rejoin_instance`
- `set_instance_weight`、`disable_instance`
- `diagnose_request`
- `plan_change`、`plan_rollback`、`list_changes`、`get_change`
- `apply_approved_change`
- `list_certificates`、`issue_certificate`
- `list_audit_events`

工具目录按当前 Key 的动作和范围裁剪；每次调用仍重新授权。MCP 不提供 Key 创建、计划批准、Shell、SSH、Docker、Kubernetes、数据库或私钥工具。

### 2.7 证书

- 静态 PEM 在启动时校验证书和私钥，并作为默认 SNI 证书。
- ACME HTTP-01 由后台、REST 或 MCP 显式触发，成功后原子热切换，不重启网关。
- ACME 账户和证书私钥使用 XChaCha20-Poly1305 加密后写入 SQLite。
- 主密钥位于独立普通文件；Unix 上拒绝 group 或 others 可读写的文件。
- 后台允许对当前证书再次显式签发；Prometheus 暴露过期、30 天内到期和最早到期时间。

续期由用户或脚本控制。v0.3 不做自动调度、DNS-01、证书上传或 DNS 提供商集成。

### 2.8 可观测性与备份

- `/healthz` 是公开的进程探针；`senixd healthcheck` 和容器 HEALTHCHECK 使用同一入口。
- `/metrics` 需要带全局 `metrics.read` 的 Bearer Key。
- 指标覆盖请求、响应状态类别、代理错误、配置版本、实例状态、普通和长连接在途数、证书到期风险。
- 指标不使用 Host、域名、URL、Key 或请求体标签，避免秘密泄漏和高基数。
- 请求诊断从运行 Snapshot 输出结构化路由证据，不依赖访问日志猜测。
- SQLite 使用 WAL、`synchronous=FULL`、忙等待、完整性检查和明确 Schema 版本。
- `backup create` 在线创建一致性备份；`verify` 校验 Schema、完整性和全部加密材料；`restore` 只创建新库，不覆盖现场文件。
- 备份文件以 `0600` 原子落盘，主密钥必须单独备份。

## 3. 外部脚本部署协议

Senix 不维护整套发布计划。推荐脚本流程：

```text
drain(instance, timeout, idempotency_key)
  -> poll(operation_id)
  -> script deploys the application instance
  -> rejoin(instance, new_generation, initial_weight)
  -> script observes application evidence
  -> set_weight(instance, next_weight)
```

代次变化会建立新的上游 Peer 身份，避免复用上一代部署的连接池。Senix 不执行部署，不自动判定灰度结果，也不自行扩大流量。

## 4. 公开接口

稳定管理接口位于 `/api/v1`：

```text
POST   /api/v1/auth/login
GET    /api/v1/auth/session
DELETE /api/v1/auth/session

GET    /api/v1/instances
GET    /api/v1/instances/{id}
POST   /api/v1/instances/{id}/drain
GET    /api/v1/operations/{operation_id}
POST   /api/v1/instances/{id}/rejoin
PATCH  /api/v1/instances/{id}/weight
POST   /api/v1/instances/{id}/disable

GET    /api/v1/config
GET    /api/v1/changes
POST   /api/v1/changes/plan
GET    /api/v1/changes/{id}
POST   /api/v1/changes/{id}/approve
POST   /api/v1/changes/{id}/apply
POST   /api/v1/snapshots/{version}/rollback-plan

POST   /api/v1/diagnostics/requests
GET    /api/v1/credentials
POST   /api/v1/credentials
DELETE /api/v1/credentials/{id}
GET    /api/v1/certificates
POST   /api/v1/certificates/issue
GET    /api/v1/audit-events
GET    /metrics
```

错误响应包含稳定 `code`、面向人的 `message` 和结构化 `evidence`。v1 新增字段必须向后兼容；删除字段或改变语义需要新主版本。

## 5. 运行与交付

- 正式构建目标为 Linux x86_64、Linux ARM64、macOS x86_64 和 macOS ARM64。
- GitHub Release 提供压缩包和 SHA-256 清单；安装脚本先校验再写入目标目录。
- GHCR 镜像以非 root UID 10001 运行，内置进程健康检查。
- Docker Demo 包含两个真实后端、持久化卷、Owner 初始化和代理验证。
- CI 在 Rust 1.88 上运行格式检查、严格 Clippy、全部测试、前端构建、嵌入产物校验和交付文件检查。
- RustSec 定期扫描依赖；Dependabot 覆盖 Cargo、npm、Docker 和 GitHub Actions。

Senix 收到 `SIGTERM` 后停止接收新连接，给已有连接配置的宽限时间，再等待运行时退出。超过期限的长连接仍可能被终止。v0.3 不承诺单机进程间的既有连接迁移，也不内置多节点高可用。

## 6. 验收要求

发布前必须满足：

1. `cargo fmt --all -- --check`、严格 Clippy 和全部测试通过。
2. 真实子进程 E2E 覆盖 HTTP/TLS 代理、主动健康、摘流与重启恢复、登录、Key Scope、审计、MCP、配置批准、证书恢复、指标和备份恢复。
3. Release 四个平台打包成功，校验清单与安装脚本在 Linux 实机通过。
4. 真实 Linux 隔离实例验证 HTTP、HTTPS、管理探针、受保护指标、在线备份和升级后恢复。
5. GitHub Dependabot 和 RustSec 不留已知高危或中危未处理告警。

性能结果必须写明硬件、请求大小、上游延迟、连接方式、TLS、并发数、压测工具和完整命令。单个峰值数字不构成通用性能承诺。

## 7. 明确不做

- Nginx 配置生成模式或 Nginx 兼容运行时。
- 自动执行 SSH、Shell、Docker 或 Kubernetes 发布命令。
- 自动编排实例发布顺序、自动判断灰度成功或自动扩流。
- WAF、API 计费、完整身份平台和服务网格。
- 任意 Rust 动态库加载或未经隔离的请求脚本。
- 在单节点 SQLite 上虚构多节点一致性或网关集群管理。

## 8. 后续候选，不属于 v0.3 承诺

按需求证据再决定，不预建空入口：

- 独立 Service 与 BackendPool 领域对象，以及 Service Scope。
- 方法、请求头和更丰富路径条件；重定向、改写与 Header 动作。
- 被动健康信号、OpenTelemetry 和外部事件通知。
- 管理面内置 TLS、完整 systemd 安装单元和恢复演练工具。
- Docker 标签发现、旧版 Senix 与常见 Nginx 配置的一次性导入。
- 经过超时、并发、熔断和失败策略约束的 HTTP 或 Unix Socket Adapter。
- h2c、mTLS、DNS-01、手动证书上传和更多协议能力。

Pingora 当前提供编译期 Rust Module 接口，但没有稳定动态插件 ABI。若以后提供扩展，优先使用外部进程协议；WASM 必须另行定义资源配额、宿主能力、ABI 版本和失败隔离。

## 9. 已确认决策

- Senix 是基于 Pingora 的独立 Rust 网关，不是 Pingora/Pingap 插件。
- 数据面请求无状态，控制面状态持久化。
- 外部脚本控制部署顺序和灰度判断，Senix 只提供幂等、可审计的流量原语。
- AI 使用 MCP 和受限 Key，不直接控制服务器，也不能绕过 Owner 批准。
- 续期由用户或脚本显式触发，Senix 只提供签发入口和到期证据。
- 当前单节点，不宣称内置高可用。
