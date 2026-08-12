# ADR-0001：v0.1 纵切的 Module 与 Seam

- 状态：已接受
- 日期：2026-08-11

## 决策

第一阶段只建立四个对调用者有直接价值的深 Module。

### ConfigEngine

Interface 负责 `plan`、`approve`、`apply`、`plan_rollback` 和读取当前 Snapshot。它隐藏配置校验、差异生成、批准有效期、版本分配、SQLite 持久化和原子发布顺序。

### TrafficController

Interface 负责 `begin_drain`、`drain_status`、实例 `status`、`rejoin`、`set_weight` 和 `disable`。`begin_drain` 立即关闭新请求入口并返回持久化 `operation_id`；`drain_status` 返回普通在途数、长连接数、截止时间与 `DRAINING`、`DRAINED` 或 `DRAIN_TIMEOUT`。它隐藏单实例保护、实例流量状态、在途请求计数、幂等处理、持久化和连接池代次。

### GatewayRuntime

Interface 负责发布不可变快照、接收健康结果，以及为一次请求取得上游租约。租约离开作用域时自动减少在途请求数。Pingora Adapter 只负责协议代理，不重新实现路由、健康或摘流规则。

### DiagnosticEngine

Interface 接受一次请求探针，返回结构化证据。它读取与数据面相同的快照和实例状态，不从日志反推运行事实。

## 已确认的测试 Seam

测试只跨越以下公开 Interface：

1. `ConfigEngine`：无效配置不能发布；有效快照能原子切换和回滚。
2. `TrafficController`：摘流后不再产生新租约；旧租约结束后状态变为已摘流；重启后状态不丢失；新代次可以回接。
3. `GatewayRuntime`：路由选择和权重通过实际请求结果观察，不检查内部集合或计数器实现。
4. `DiagnosticEngine`：诊断返回稳定的证据步骤和失败位置。
5. 系统 Seam：通过真实 HTTP 请求验证 Pingora 双后端代理和控制接口。

SQLite 使用临时真实数据库测试。上游使用本地 HTTP 进程测试。HTTP/TCP 主动健康检查也通过真实监听端口和系统 Seam 验证，不模拟探测结果。只在 Pingora、文件系统、网络和时间等系统 Seam 使用 Adapter，不模拟内部 Module。

## 约束

- 数据面请求不查询 SQLite。
- 所有请求只读取一个不可变运行快照。
- 流量状态与健康状态正交。
- 主动检查的实例从 `UNKNOWN` 开始，达到成功阈值后才参与选路。
- 健康恢复不能覆盖人工摘流或禁用状态。
- 会让任一路由失去最后一个可用后端的摘流默认拒绝；只有显式 `force=true` 可以绕过。
- 摘流超时只报告 `DRAIN_TIMEOUT`，不主动终止连接，也不执行部署命令。
- 回接必须等待旧代次完全摘流；健康绕过必须保留原健康状态，并以 `health_override` 单独展示和持久化。
- 摘流先关闭新租约入口，再持久化期望状态。
- 持久化失败时返回明确错误，不能把实例悄悄恢复为接流状态。
- 外部脚本决定发布顺序和灰度结果，Senix 不执行部署命令。

## 目录

```text
crates/
  senix-core/       四个核心 Module 与 SQLite Adapter
  senix-pingora/    Pingora Adapter
  senixd/           单二进制、主动健康检查和管理 HTTP Adapter
```

## 暂不处理

MCP、证书、账号、完整变更审批、React 后台、Docker 自动发现和外部插件 Adapter 不属于这次纵切。
