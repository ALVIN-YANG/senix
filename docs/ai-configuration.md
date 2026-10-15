# 通过 AI 安全修改网关配置

本流程让 AI 读取完整配置、提出变更，并在 Owner 批准后应用。AI 只访问 Senix MCP；部署应用仍由用户或外部脚本负责。

> `get_current_config` 和权限模板是当前源码的新功能，尚未发布。默认 Docker Demo 固定在已发布的 `0.3.1`，只用于体验原有代理和管理能力。验证本页完整流程时，请先构建当前源码。

## 1. 启动当前源码

需要 Git、Docker、Docker Compose v2 和 curl，并能访问构建使用的镜像及包仓库。以下命令在仓库根目录执行。首次构建需要下载和编译依赖，不计入五分钟操作体验。

```bash
docker build -t senix:local .
SENIX_IMAGE=senix:local SENIX_DEMO_PULL=0 ./scripts/demo.sh
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
保留所有其他路由、后端和字段，并调用 plan_change。
向我展示 change_id、base_version、candidate_digest、完整候选配置和差异。
先等待我在 Senix 管理后台批准，不要应用。
```

`get_current_config` 返回 `{ "version": 1, "config": { ... } }`；实际版本以返回值为准。把 `config` 对象复制为候选配置，只修改目标字段，再传给 `plan_change` 的 `candidate`。不要把整个 Snapshot 当成候选配置。

计划创建后，原域名仍能代理：

```bash
curl -f -H 'Host: demo.senix.local' http://127.0.0.1:8080/
```

计划生成的 `base_version` 必须等于刚读取的 `version`。如果不同，说明读取和规划之间有人发布了新配置：放弃这份计划，重新读取后生成候选配置，避免覆盖别人的变更。

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
| `STALE_PLAN` | 重新读取当前配置、合并修改、创建新计划，再批准 |

`get_current_config` 需要全局 `change.read`。只读 Key 看不到规划或应用工具，直接调用也会被拒绝；实例范围 Key 不能读取全局配置。吊销立即生效，客户端可能仍显示旧工具列表，但调用仍会重新鉴权。

## 自动化验收与人工体验

仓库的真实进程 E2E 使用 MCP 读取、规划和应用，Owner 测试账号通过管理 API 批准，并向真实后端发送请求；同时覆盖未批准、过期基线、权限隔离和吊销。运行：

```bash
cargo +1.88 test --locked -p senixd --test gateway_e2e
```

这些测试验证协议和网关行为，不证明任意模型都能正确生成候选配置。实际客户端仍需按本页执行一次完整操作，并由人检查计划内容。
