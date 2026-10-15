# 通过 AI 安全修改网关配置

**简体中文** | [English](./ai-configuration.en.md)

本流程让 AI 读取完整配置、提出变更，并在 Owner 批准后应用。AI 只访问 Senix MCP；部署应用仍由用户或外部脚本负责。

> 本流程使用 `v0.4.0`，包含 `get_current_config`、规划版本校验、配置 JSON Schema 和权限模板。旧 MCP 调用方须补 `expected_version`，见 [升级说明](releases/v0.4.0.md)。

语言说明：`v0.4.0` 发布版后台为中文。主分支源码已增加中英文切换与语言记忆，尚未发布；下文继续以已发布版本为准。[源码后台验收](validation/2026-10-15-admin-languages.md) 包含英文界面的相同操作流程。

## 1. 启动 v0.4.0

需要 Git、Docker、Docker Compose v2 和 curl，并能访问镜像仓库。以下命令在 `v0.4.0` 仓库根目录执行（克隆命令见 README），无需本地编译。

```bash
SENIX_IMAGE=ghcr.io/alvin-yang/senix:0.4.0 ./scripts/demo.sh
```

脚本输出管理地址、Owner 用户名、首次生成的密码和真实代理响应。管理端口只绑定本机；端口 `8080`、`9080` 和 Docker 子网 `172.28.0.0/24` 必须可用。重跑会保留配置和账号，使用首次密码登录，不会重置现场数据。

如果默认端口被占用，可在同一命令前增加 `SENIX_DEMO_PROXY_PORT=18080 SENIX_DEMO_ADMIN_PORT=19080`。脚本的探针和输出会使用新端口；下文后台地址、MCP URL 和 curl 地址也应替换为对应端口。停止时保留相同的环境变量。

如果已经运行过旧版 Demo，先执行脚本输出的停止命令，再用上述命令启动。数据库有配置后会恢复已保存的 Snapshot，修改示例 JSON 不会覆盖它。

## 2. 签发一把配置协作 Key

打开 `http://127.0.0.1:9080/admin/`，用脚本输出的 Owner 账号登录。进入“访问 Key”，点击“生成 Key”，选择用途模板：

| 模板 | 允许动作 | 边界 |
| --- | --- | --- |
| 只读诊断 | `instance.read`、`diagnostics.read`、`change.read` | 查看状态、诊断和当前配置 |
| 配置提案 | 只读诊断 + `change.plan` | 可以创建配置或回滚计划，不能应用 |
| 配置协作 | 配置提案 + `change.apply` | 只能应用 Owner 已批准的精确计划 |

本演示选择“配置协作”，名称填 `local-config-agent`，有效期选择 24 小时。诊断和配置动作目前需要全局范围，因此模板会选中“所有当前和未来实例”；它们不提供实例隔离。核对权限后签发，完整 Key 只展示一次。修改动作或范围后，表单会转为自定义权限。

三种模板均不能批准计划、签发证书、摘流、禁用实例或运行服务器命令。希望把提案和执行交给不同客户端时，可选择“配置提案”，并另行签发仅包含 `change.read`、`change.apply` 的全局 Key。

## 3. 接入 Cursor

在 Cursor 的全局 `~/.cursor/mcp.json` 中合并下面的 `senix` 条目，保留已有服务器：

```json
{
  "mcpServers": {
    "senix": {
      "url": "http://127.0.0.1:9080/mcp",
      "headers": {
        "Authorization": "Bearer ${env:SENIX_API_KEY}"
      }
    }
  }
}
```

`SENIX_API_KEY` 必须存在于 **Cursor 进程**的环境中。可在终端静默读取 Key、导出变量，完全退出原有 Cursor 进程后，从同一个终端重新启动 Cursor。仅在另一个终端执行 `export` 不会改变已运行进程的环境。不要把真实 Key 写入仓库或聊天消息。

Cursor 支持 HTTP `headers` 和环境变量插值；远程 HTTP 配置不支持 `envFile`。配置方式见 [Cursor 官方 MCP 文档](https://prod.cursor.com/docs/mcp)。界面位置以所用客户端版本为准。

启用 Senix 后，确认可见 `get_current_config`、`plan_change`、`get_change` 和 `apply_approved_change`。工具列表按 Key 权限裁剪；客户端对工具调用的确认，与 Senix Owner 对配置计划的批准是两道独立检查。

## 4. 生成计划，保留其他配置

给 AI 输入：

```text
使用 Senix MCP 的 get_current_config 读取当前配置。
只把 demo-route 的 host 从 demo.senix.local 改为 preview.senix.local，
保留所有其他路由、后端和字段，调用 plan_change，
expected_version 使用这次读取的 version，candidate 使用完整 config。
向我展示 change_id、base_version、candidate_digest、完整候选配置和差异。
先等待我在 Senix 管理后台批准，不要应用。
```

`get_current_config` 返回 `{ "version": 1, "config": { ... } }`；实际版本以返回值为准。把 `config` 对象复制为候选配置，只修改目标字段，再传给 `plan_change` 的 `candidate`，同时将读取的 `version` 传给必填参数 `expected_version`。不要把整个 Snapshot 当成候选配置。`tools/list` 提供路由、后端、TLS 和健康检查的配置 JSON Schema，字段定义来自实际配置类型。

调用结构如下；省略部分需要换成读取并修改后的完整配置：

```text
plan_change({"expected_version": 1, "candidate": {"routes": [...]}})
```

计划创建后，原域名仍能代理：

```bash
curl -f -H 'Host: demo.senix.local' http://127.0.0.1:8080/
```

如果读取和规划之间有人发布新配置，服务端返回 `CONFIG_VERSION_MISMATCH`，在 `evidence` 中给出 `expected_version` 和 `current_version`，不会创建计划。必须重新读取完整配置并合并本次修改，再用新读取的版本规划；不能只把旧候选的版本号改成最新值。校验和计划持久化与配置发布共用同一把锁。

**MCP 调用兼容性：** 旧调用方必须补传 `expected_version`；缺参、字段类型错误或配置包含未知字段时由 MCP SDK 返回 `isError: true` 和参数反序列化错误说明，尚未进入规划。结构正确但业务校验不通过的配置仍生成带 `issues` 的计划，不能批准。REST v1 仍接收原有的完整配置请求体，不新增版本前置条件；`plan_rollback` 保持原有契约。

## 5. Owner 批准，AI 应用

在管理后台“配置变更”中，按 `change_id` 找到计划，核对摘要、完整候选配置、差异和校验结果，再点击批准。批准只对这一份内容有效，15 分钟后失效。

然后告诉 AI：

```text
我已在 Senix 后台批准刚才的计划。
先用 get_change 检查同一个 change_id 的状态、摘要和批准有效期，
再调用 apply_approved_change。最后读取 get_current_config，确认版本和修改结果。
```

用真实请求验证新域名：

```bash
curl -f -H 'Host: preview.senix.local' http://127.0.0.1:8080/
```

这个 Demo 原来只有一条路由，修改后旧域名应不再匹配。实际配置有其他路由时，也要检查它们仍可访问。恢复时可让 AI 对原版本创建 `plan_rollback`，再走相同的 Owner 批准和应用流程。

保留修改后的配置并重跑启动脚本时，增加 `SENIX_DEMO_HOST=preview.senix.local`，让代理探针检查当前域名；原有的镜像和端口覆盖参数也需保留。

## 6. 失败时如何处理

| 返回结果 | 下一步 |
| --- | --- |
| `AUTHENTICATION_REQUIRED` | 检查 Key、客户端进程环境、有效期及吊销状态 |
| `FORBIDDEN` | 核对动作和全局范围；不要直接改成全权限 Key |
| `CHANGE_APPROVAL_REQUIRED` | 由 Owner 在后台核对并批准同一计划 |
| `CHANGE_APPROVAL_EXPIRED` | Owner 重新核对并批准，不能复用过期批准 |
| `CONFIG_VERSION_MISMATCH` | 重新读取并合并修改，用读取版本重新规划；本次未创建计划 |
| `STALE_PLAN` | 重新读取当前配置、合并修改、创建新计划，再批准 |

`get_current_config` 需要全局 `change.read`。只读 Key 看不到规划或应用工具，直接调用也会被拒绝；实例范围 Key 不能读取全局配置。吊销立即生效，客户端可能仍显示旧工具列表，但调用仍会重新鉴权。

## 自动化验收与人工体验

仓库的真实进程 E2E 使用 MCP 读取、规划和应用，Owner 测试账号通过管理 API 批准，并向真实后端发送请求；同时覆盖读取到规划之间的发布冲突、重读后保留新增路由、缺少版本、配置 Schema、REST v1 兼容、未批准、过期基线、权限隔离和吊销。运行：

```bash
cargo +1.88 test --locked -p senixd --test gateway_e2e
```

这些测试验证协议和网关行为，不证明任意模型都能正确生成候选配置。实际客户端仍需按本页执行一次完整操作，并由人检查计划内容。
