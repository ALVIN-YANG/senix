# MCP 配置规划版本校验验收

日期：2026-10-15。范围：基于 `08b1816` 的源码增量，尚未发布 Release 或镜像。

## 修复的问题

在此前源码 `af40d95` 的真实 HTTP MCP 流程中，AI 读取 v1，Owner 用 REST 发布新增路由的 v2，AI 再提交基于 v1 的完整候选。旧实现会以 v2 创建计划，并把 Owner 新增路由列为删除。本次复现没有批准或应用这份危险计划，实际配置仍为 Owner 的 v2；[变更前证据](mcp-version-guard/2026-10-15/before.json) 已保留。

MCP `plan_change` 现在必填 `expected_version`。`ConfigEngine::plan_at_version` 在与批准、发布共用的锁内检查当前版本，冲突时返回 `CONFIG_VERSION_MISMATCH`，证据包含期望版本和当前版本，且不保存计划。调用方须重新读取并合并修改，不能只改旧候选的版本号。

候选参数直接使用 `GatewayConfig`，从实际配置类型派生 [公开 JSON Schema](mcp-version-guard/2026-10-15/plan-change-input-schema.json)，包括路由、后端、TLS、ALPN、健康检查、默认值与未知字段限制。只增加现有锁定的 `schemars 1.2.2` 直接依赖，没有增加另一套配置 DTO。

## 验证结果

- Rust 1.88 工作区 81 项测试通过，含 27 项真实进程 E2E；格式检查与严格 Clippy 通过。新增核心测试覆盖版本 0、旧版本、未来版本、当前版本与拒绝时不保存计划；真实 MCP E2E 覆盖公开 Schema、缺参、结构错误、并发发布、重读合并、Owner 批准和真实请求。
- Linux ARM64 release 镜像 `sha256:6bd0271eeed5461a17d875c55244fd79b53262a91c78adeac9051337ae36ad97` 构建成功。三次独立 Compose/MCP 流程均完成：AI 读 v1 → Owner 新增路由发布 v2 → 旧候选被拒绝且计划列表不变 → 重读并保留新增路由 → 未批准应用被拒绝 → 测试 Owner 通过管理 API 批准 → MCP 应用 v3 → 重复应用返回 v3。
- 三次请求检查均为：`ai-change.senix.local` 200、`owner-added.senix.local` 200、旧 `demo.senix.local` 404。新增路由的完整配置保持一致。见 [第 1 次](mcp-version-guard/2026-10-15/after-1.json)、[第 2 次](mcp-version-guard/2026-10-15/after-2.json)、[第 3 次](mcp-version-guard/2026-10-15/after-3.json)。
- REST v1 仍用原有完整配置请求体规划、批准和应用，没有新增版本信封或必填参数；`plan_rollback` 保持原有契约，现有回滚、旧计划、权限、吊销和批准检查继续通过。
- `cargo audit --deny yanked` 通过，没有漏洞和已撤回依赖；仍有 `daemonize`、`derivative`、`unsafe-libyaml` 未维护和 `lru 0.16.4` 的 `RUSTSEC-2026-0253` 提示。未添加忽略规则。

## 兼容性与证据边界

旧 MCP 调用方必须补 `expected_version`。当前 SDK 对缺参、字段类型错误和未知配置字段返回工具错误 `isError: true` 及反序列化说明；它们不会进入领域规划。业务校验问题仍出现在计划 `issues` 中，并阻止批准。中英文 README、指南、llms.txt、需求和 ADR 已同步。

首次容器验收在 Demo 启动阶段退出 1，临时脚本当时未保存输出，因此原因未确认。Docker 事件只能确认两个引导命令退出 0、服务容器已启动，不能确定后续启动失败原因。随后补齐脱敏启动日志，连续三次完整流程通过，保留了 [首次事件](mcp-version-guard/2026-10-15/first-start-events.json) 和后续启动输出。不能据此宣称已修复这一偶发启动问题；现有探针超时修复也不能被当作它的已知根因。

本记录验证当前源码的协议、核心规则和真实代理结果；测试 Owner 通过管理 API 批准，不代表特定 Cursor/模型的任务成功率验收。没有生产部署、Release 或镜像发布。

[原始 Rust 输出](mcp-version-guard/2026-10-15/rust-tests.txt) · [复现脚本](mcp-version-guard/2026-10-15/probes.zip) · [SHA-256](mcp-version-guard/2026-10-15/SHA256SUMS)。脚本仅用于隔离的临时 Compose 项目；变更后脚本在仓库根目录运行，先构建 `senix:version-guard`，依赖 Python 3、Docker、Compose 和 18080/19080 可用。测试凭据在内存生成，公开日志已脱敏，测试容器和卷已清理。
