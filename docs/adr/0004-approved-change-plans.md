# Approved immutable plans are the only management write path

Senix 把候选配置持久化为绑定当前 Snapshot 的不可修改 Change Plan，Owner 的 Approval 只对这份精确内容生效 15 分钟，Apply 时必须再次确认基线没有变化。回滚也创建 Rollback Plan，不保留绕过审批的管理写入口；MCP 可以规划并在获得独立 `change.apply` 授权后应用已批准计划，但不能批准计划。这样外部脚本和 AI 仍可自动执行，同时不会把“能调用工具”扩大成“能自行批准配置”。
