# Senix

Senix 是一个基于 Rust 和 Pingora 的独立网关，不依赖 Nginx、Go 服务或外部数据库。当前仓库是可运行的 v0.1 纵切，还不是完整发布版。

目前已经实现：

- Pingora 双后端 HTTP 代理和平滑加权轮询。
- 不可修改的配置计划、结构化差异、15 分钟 Owner 批准、原子发布和回滚计划。
- 持久化摘流操作、在途请求查询、超时状态、回接、权重调整和禁用。
- 流量写操作幂等，摘流与禁用状态可跨重启恢复。
- 单实例保护、显式强制操作和结构化错误证据。
- HTTP/TCP 主动健康检查，支持间隔、超时、成功阈值和失败阈值。
- 不健康实例自动退出新请求选择，恢复健康不会覆盖人工摘流或禁用状态。
- 结构化路由诊断和最小 REST 控制接口。
- 本机两步 Owner 引导、Argon2id 密码、短期浏览器会话，以及只存摘要的受限 API Key。
- REST 与 MCP 共用的默认拒绝授权、结构化错误和无秘密审计。
- 无会话 Streamable HTTP MCP；工具清单按当前 Key 的能力裁剪。
- React 管理台中的实例摘流/回接/权重/禁用、配置编辑、批准队列和 Snapshot 版本视图。

暂未实现 MCP stdio 桥、Service 范围、被动健康信号、证书、完整服务建模、长连接分类与强制收尾、多节点和插件 Adapter。完整边界见 [需求文档](docs/requirements.md)，模块与安全决策见 [docs/adr](docs/adr)。

## 目录

```text
crates/senix-core      配置、流量、运行时和诊断 Module
crates/senix-mcp       MCP Streamable HTTP Adapter
crates/senix-pingora   Pingora Adapter
crates/senixd          单进程入口、健康检查和管理 HTTP Adapter
docs                   需求与架构决策
examples               可运行配置
```

## 构建

需要 Rust 1.88、C/C++ 编译器和 CMake。Pingora 的压缩依赖会在构建时编译 zlib-ng。

```bash
# macOS
brew install cmake

# Ubuntu/Debian
sudo apt-get install build-essential cmake

cargo build --release --locked -p senixd
```

## 运行

先在 `4101` 和 `4102` 启动两个 HTTP 后端。第一次运行前，在服务器本机完成两步引导。先创建一次性 Owner Credential：

```bash
cargo run -p senixd -- credential bootstrap \
  --db /tmp/senix.db \
  --label local-owner
```

然后通过标准输入创建唯一 Owner Account，避免密码出现在进程参数或 Shell 历史：

```bash
printf '%s' "$SENIX_OWNER_PASSWORD" | cargo run -p senixd -- owner bootstrap \
  --db /tmp/senix.db \
  --username admin \
  --password-stdin
```

账号创建会在同一个 SQLite 事务中立即撤销前一步的 Owner Credential。以后通过 `http://127.0.0.1:9080/admin/` 登录管理后台。忘记密码时只能在服务器本机重置；重置会让所有浏览器会话失效：

```bash
printf '%s' "$SENIX_NEW_OWNER_PASSWORD" | cargo run -p senixd -- owner reset-password \
  --db /tmp/senix.db \
  --password-stdin
```

完成引导后启动网关：

```bash
cargo run -p senixd -- \
  --listen 127.0.0.1:8080 \
  --admin-listen 127.0.0.1:9080 \
  --db /tmp/senix.db \
  --config examples/gateway.json
```

```bash
curl -H 'Host: example.test' http://127.0.0.1:8080/
```

数据库为空时必须提供 `--config`。数据库已有配置快照时，Senix 恢复最新快照并忽略启动配置，避免重启覆盖已生效状态。

Docker 镜像使用非 root 用户，并且只运行 `senixd`：

```bash
docker build -t senix:dev .
docker run --rm \
  -v senix-data:/var/lib/senix \
  senix:dev credential bootstrap \
  --db /var/lib/senix/senix.db \
  --label local-owner

printf '%s' "$SENIX_OWNER_PASSWORD" | docker run --rm -i \
  -v senix-data:/var/lib/senix \
  senix:dev owner bootstrap \
  --db /var/lib/senix/senix.db \
  --username admin \
  --password-stdin

docker run --rm \
  -p 8080:8080 \
  -p 127.0.0.1:9080:9080 \
  -v "$PWD/gateway.json:/etc/senix/gateway.json:ro" \
  -v senix-data:/var/lib/senix \
  senix:dev
```

React 管理后台使用 Owner Account 登录，REST 脚本和 MCP 使用 `Authorization: Bearer <key>`；MCP 不接受浏览器 Cookie。管理端口默认仍应只绑定本机或私网；当前未内置管理面 TLS，不要把 `9080` 直接暴露到公网。通过受信 TLS 入口访问后台时需增加 `--admin-secure-cookie`。容器配置中的后端地址必须能从容器网络访问。

## 管理后台

后台产物嵌入 `senixd`，不需要单独部署前端。当前页面支持实例流量/健康状态、手动摘流、按新代次回接、权重调整、二次确认禁用、完整配置规划、Owner 批准、受限 Key 管理和审计查看。最后一个可用后端和不健康回接不会被静默绕过；需要操作者显式勾选强制操作。浏览器 Cookie 为 HttpOnly、SameSite=Strict；写操作还要求同源 CSRF 头。退出登录与本机密码重置都会使旧浏览器会话立即失效。

修改前端后需重新生成嵌入产物：

```bash
cd web
npm ci --ignore-scripts
npm run build
```

## Credential 与审计

Owner 可以在后台或 REST 生成只允许指定动作和实例的 Key。Key 只展示一次，数据库保存加盐摘要；签发和审计在同一个 SQLite 事务中提交。后台是推荐入口；REST 调用需先通过 `/api/v1/auth/login` 获取 Owner Cookie，并在写请求中携带 `X-Senix-CSRF: 1`：

```bash
curl -X POST \
  -b "$SENIX_OWNER_COOKIE" \
  -H 'X-Senix-CSRF: 1' \
  -H 'Content-Type: application/json' \
  -d '{
    "label":"deploy-instance-a",
    "actions":["instance.read","instance.drain","instance.rejoin","instance.set_weight"],
    "instance_ids":["instance-a"],
    "all_resources":false
  }' \
  http://127.0.0.1:9080/api/v1/credentials
```

`GET /api/v1/credentials` 只返回元数据；`DELETE /api/v1/credentials/{id}` 立即撤销 Key。`GET /api/v1/audit-events` 返回操作者、动作、资源、结果和风险，不保存 Key、Authorization 头或完整请求体。轮换由用户或外部脚本按“签发新 Key、切换调用方、撤销旧 Key”完成。

当前动作名为 `instance.read`、`instance.drain`、`instance.rejoin`、`instance.set_weight`、`instance.disable`、`diagnostics.read`、`change.plan`、`change.read`、`change.apply` 和 `audit.read`。`change.approve` 只允许 Owner，不能签发给 API Key。Service 还不是当前运行模型中的真实实体，因此首版只承诺全局或 Instance 范围；配置变更动作需要全局范围。

## MCP

Streamable HTTP MCP 位于 `http://127.0.0.1:9080/mcp`，每个请求都使用后台生成的 Bearer Key。MCP 不保存协议会话；同一 Key 在 REST 和 MCP 上得到相同授权与审计结果。`tools/list` 会按 Key 的动作和实例范围隐藏无权使用的工具，实际调用时仍会再次授权。

当前暴露 `list_instances`、`get_instance_health`、`drain_instance`、`get_drain_status`、`rejoin_instance`、`set_instance_weight`、`disable_instance`、`diagnose_request`、`plan_change`、`plan_rollback`、`list_changes`、`get_change`、`apply_approved_change` 和 `list_audit_events`。MCP 没有批准工具，也不能修改已生成计划；只有 Owner 批准精确候选内容后，带 `change.apply` 的 Key 才能应用。MCP 不提供 Key 创建、Shell、SSH、Docker 或 Kubernetes 工具。

MCP 默认只接受 `localhost`、`127.0.0.1` 和 `::1` 的 Host。通过私网域名访问时必须显式加入允许列表；浏览器客户端还应配置 Origin：

```bash
senixd \
  --mcp-allowed-host gateway.internal \
  --mcp-allowed-origin https://admin.example.com \
  --listen 127.0.0.1:8080 \
  --admin-listen 0.0.0.0:9080 \
  --db /var/lib/senix/senix.db \
  --config /etc/senix/gateway.json
```

## 健康检查

健康检查配置在实例上。HTTP 探测接受 `200` 到 `399`，TCP 探测要求在超时前完成连接。启用主动检查的实例启动时为 `UNKNOWN`，达到成功阈值后才接收请求。

```json
{
  "health_check": {
    "protocol": "http",
    "path": "/health",
    "interval_ms": 5000,
    "timeout_ms": 1000,
    "healthy_threshold": 2,
    "unhealthy_threshold": 3
  }
}
```

## 流量控制

```bash
# 摘流
curl -X POST \
  -H "Authorization: Bearer $SENIX_API_KEY" \
  -H 'Content-Type: application/json' \
  -H 'Idempotency-Key: deploy-42-drain-a' \
  -d '{"timeout_ms":60000,"force":false}' \
  http://127.0.0.1:9080/api/v1/instances/instance-a/drain

# 使用返回的 operation_id 查询摘流状态
curl -H "Authorization: Bearer $SENIX_API_KEY" \
  http://127.0.0.1:9080/api/v1/operations/OPERATION_ID

# 查询实例流量与健康状态
curl -H "Authorization: Bearer $SENIX_API_KEY" \
  http://127.0.0.1:9080/api/v1/instances/instance-a

# 使用新代次和 5% 权重回接
curl -X POST \
  -H "Authorization: Bearer $SENIX_API_KEY" \
  -H 'Content-Type: application/json' \
  -H 'Idempotency-Key: deploy-42-rejoin-a' \
  -d '{"generation":2,"weight":5,"force":false}' \
  http://127.0.0.1:9080/api/v1/instances/instance-a/rejoin
```

摘流会返回 `operation_id`、截止时间、普通在途数和长连接数。普通请求默认等待 60 秒；到期后返回 `DRAIN_TIMEOUT`，Senix 不会强杀连接。当前 HTTP 纵切尚未识别 WebSocket、SSE 和 gRPC 长连接，因此 `long_lived_in_flight` 固定为 `0`。

会让任一路由失去最后一个可用后端的摘流默认返回 `409 LAST_AVAILABLE_BACKEND`，脚本必须明确传入 `force=true`。这里的强制只绕过单实例保护，不会终止已有连接。

回接必须等待旧代次完全摘流。不健康实例默认拒绝回接；`force=true` 会留下 `health_override=true` 并允许选路，但原始健康状态仍保持 `UNHEALTHY`，并记录高风险审计。

同一个幂等键只能用于同一个实例和同一种操作。重复摘流返回相同 `operation_id`，不会重新修改当前代次。领域错误响应包含稳定 `code`、可读 `message` 和结构化 `evidence`。

## 诊断与测试

```bash
curl -X POST \
  -H "Authorization: Bearer $SENIX_API_KEY" \
  -H 'Content-Type: application/json' \
  -d '{"host":"example.test","path":"/api"}' \
  http://127.0.0.1:9080/api/v1/diagnostics/requests

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
```

端到端测试会启动真实 HTTP 后端和 `senixd` 子进程，验证持久化摘流、状态恢复、主动健康检查、Owner 登录、Key 范围/撤销/审计，以及 MCP 能力裁剪、越权拒绝、精确批准后应用和幂等重放。
