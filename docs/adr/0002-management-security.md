# ADR-0002: One security boundary for every management adapter

- Status: accepted
- Date: 2026-08-11

## Context

Senix 的管理入口会同时面向控制台、部署脚本和 AI 的 MCP 客户端。如果 REST、CLI 和 MCP 各自实现权限判断，规则会漂移，审计也无法回答“谁通过哪个入口改变了什么”。管理 API 当前没有认证，不能安全暴露。

当前领域只有 Instance，没有可用于授权的 Service 实体。先声明 Service Scope 会造成无法可靠执行的假权限。

## Decision

引入 `SecurityController` 作为管理安全的 Module Interface。它负责：

- `bootstrap_owner_key`：仅当数据库没有 Credential 时创建 Owner Credential；
- `bootstrap_owner_account` 和 `reset_owner_password`：通过本机恢复入口管理唯一 Owner Account；
- `login_owner`、`authenticate_owner_session` 和 `logout_owner`：签发、验证和失效短期浏览器会话；
- `issue_key`：签发带有效期、Management Action 和 Resource Scope 的受限 API Key；
- `authenticate`：把 Bearer Key 转换为 Principal；
- `authorize`：按任务级 Action 和资源执行默认拒绝的授权；
- `revoke`：立即撤销 Credential；
- `record_audit` 和 `list_audit`：记录并查询不可修改的 Audit Event。

REST、管理后台和 MCP 都只是 Adapter，必须调用同一个 `SecurityController`，不得自行解释 Scope 或绕过审计。`/healthz` 保持公开，除登录外的 `/api/v1/*` 路由默认需要认证，并由各用例明确授权。

Owner Credential 只允许通过本机 CLI 一次性引导。建立唯一 Owner Account 后，该引导 Credential 在同一事务中立即失效。Owner 密码使用 Argon2id 保存；浏览器使用短期 HMAC 签名 Cookie，不保存服务端会话列表。退出登录或本机重置密码会轮换签名密钥，使全部旧会话立即失效。Cookie 写操作必须携带同源 CSRF 头；API Key 和 MCP 继续只接受 Bearer，不接受浏览器 Cookie。

Owner 登录按 TCP Peer IP 使用有界的进程内失败窗口，不信任转发头。五次失败锁定十五分钟，同时最多执行两次 Argon2 校验，校验放入阻塞线程池，避免并发爆破占满管理异步运行时。

API Key 使用足够强的随机值，完整值只展示一次；持久层只保存不可逆摘要。认证和授权遇到未知状态时一律拒绝。审计不得保存密码、Cookie、API Key、Authorization 头或完整请求体。

首版 Resource Scope 只支持全局和明确的 Instance ID 集合。等 Service 成为真实领域实体并有稳定归属关系后，再扩展 Service Scope。

Key 轮换由用户或外部脚本按“签发新 Key、切换调用方、撤销旧 Key”完成，Senix 不自动决定切换时机。

## Module boundary

调用方只看见 Credential 签发结果、Principal、Grant、Management Action、Resource Scope 和 Audit Event，不接触摘要算法或数据库行。`SecurityController` 直接使用当前唯一的 SQLite Adapter；在出现第二种真实存储实现前，不增加假想的存储抽象。

## Verification seams

- `SecurityController` 测试认证、过期、撤销和 Scope 判断；
- 真实 `senixd` CLI/HTTP 系统测试覆盖本机引导、默认拒绝、Bearer 认证和稳定错误码；
- 管理后台系统测试覆盖账号登录、签名 Cookie、CSRF、退出失效和本机密码重置；
- 受限 Key 的系统测试覆盖允许动作、越权拒绝和无密钥泄漏的审计；
- 未来 MCP Adapter 复用同一组 Action 和 Resource Scope 合同。

## Consequences

管理面默认不再匿名可用，首次启动前需要执行 Credential 与 Owner Account 两步本机引导。浏览器后台和自动化入口使用不同凭据形态，但认证后进入同一个 Principal、授权和审计 Module。权限名独立于 REST 路径和 MCP 工具名，后续入口可以增加而不复制安全规则。Service Scope 延后到模型真实存在，避免首版给出虚假的隔离保证。
