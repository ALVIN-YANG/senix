# Senix Domain Language

Senix 的管理面由人、部署脚本和 AI 工具共同使用。无论入口是 REST、CLI 还是 MCP，它们都使用同一套身份、授权和审计语义。

## Identity and authority

| Term | Meaning | Not this |
| --- | --- | --- |
| Credential | 管理请求所携带、可被撤销的一份身份凭据。 | 登录会话或业务流量的认证信息。 |
| Owner Credential | 本机一次性引导创建的全权限凭据，用于建立 Owner Account；账号创建后立即失效。 | Owner Account 的长期登录凭据，或可随意复制的普通 API Key。 |
| Owner Account | 唯一的人类所有者身份，使用用户名和密码登录管理后台。 | 部署脚本或 AI 使用的 Credential。 |
| Owner Session | Owner Account 登录后得到的短期浏览器凭据；退出或重置密码会使已有会话失效。 | 可持久用于自动化的 API Key。 |
| API Key | 只在签发时展示一次的 Bearer 密钥，用来识别一个 Credential。 | 数据库中可找回的明文密码。 |
| Principal | 一个管理请求通过 Owner Session 或 API Key 认证后得到的当前身份和权限。 | 长期保存的登录凭据。 |
| Grant | 允许 Principal 对指定资源执行指定 Management Action 的授权。 | 对某个 HTTP 路径的粗粒度放行。 |
| Management Action | 稳定的任务级操作，例如读取实例、摘流、接回、调权、禁用、诊断、规划变更、管理凭据和读取审计。 | 与某个 REST 路由或 MCP 工具绑定的权限名。 |
| Resource Scope | Grant 可以影响的资源范围。当前可表达全局或一组 Instance。 | 用空集合隐式表示全部资源。 |
| Capability-shaped Tool Catalog | MCP 根据当前 Principal 的 Grant 生成的工具视图，只展示至少有一个可操作资源的任务。 | 授权边界；实际工具调用仍必须重新认证和授权。 |

Service 仍未成为当前网关模型中的真实实体，因此首版不虚构 Service Scope。等 Service 有稳定标识和归属关系后，再把它加入 Resource Scope。

## Accountability

| Term | Meaning | Not this |
| --- | --- | --- |
| Audit Event | 一条不可修改的管理行为记录，包含操作者、时间、动作、资源、结果和风险级别。 | 可能包含 API Key、Authorization 头或完整请求体的调试日志。 |
| Outcome | 管理动作成功、失败或被拒绝的结果。 | 只有 HTTP 状态码、没有领域含义的记录。 |
| Risk | 动作的影响等级；强制摘流等破坏性动作属于高风险。 | 根据调用入口推断的风险。 |

## Lifecycle

| Term | Meaning | Not this |
| --- | --- | --- |
| Bootstrap | 先在本机创建 Owner Credential，再用它建立唯一 Owner Account 的一次性过程；完成后引导 Credential 失效。 | 可远程反复调用的初始化接口。 |
| Issue | 创建受限 API Key，并只在该次响应中返回完整密钥。 | 之后仍可查询明文密钥。 |
| Revoke | 立即让一个 Credential 失效，同时保留其身份和审计关联。 | 删除所有历史记录。 |
| Rotate | 签发新 Key、切换调用方、再撤销旧 Key 的用户或脚本流程。 | 网关自动替用户决定切换时间。 |
