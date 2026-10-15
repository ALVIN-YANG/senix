# 监控接入验收记录

日期：2026-10-15。范围：`d99453c` 上的本轮未发布工作区；Linux ARM64 release 镜像 `sha256:55b61e0c0b43020214b38f922e525eeb4dabddc1ff2f21140ad53e704634f259`。本机 Docker 网络中的独立 Demo，不涉及生产账户或外部监控服务。

## 真实链路

| 组件版本 | 验证动作 | 结果 |
| --- | --- | --- |
| Prometheus 3.15.0 | 使用全局 `metrics.read` Key 抓取，查询 `up`、请求计数、直方图和正值 p95 | 通过 |
| Grafana 13.2.4 | 文件预置数据源和 8 个面板，经数据源代理查询真实快照；浏览器查看图表 | 通过，保存真实截图 |
| VictoriaMetrics 1.154.0 | 专用 scrape 配置直接抓取 Senix | `up{job="senix"}=1` |
| OTel Collector Contrib 0.162.0 | Prometheus receiver → batch → Prometheus exporter | 8889 出口包含 Senix 耗时直方图 |
| Grafana Alloy 1.20.1 | 带凭据抓取 → remote_write → VictoriaMetrics | `up{job="senix_alloy"}=1` |
| Loki 3.7.8 | Alloy 读取 Senix 真实 JSON 日志文件，发送后按事件查询 | 返回 `proxy_request_completed` 事件 |
| 权限撤销 | 测试 Owner 吊销采集 Key，继续等待 Prometheus 抓取 | `up{job="senix"}=0` |

匿名 `/metrics` 返回 401；同一采集 Key 读取 `/api/v1/config` 返回 403。没有向采集器授予配置、流量操作或 Owner 权限。Key 与测试 Owner 密码只保存在忽略的测试文件中，完成后删除。

[精简运行数据](2026-10-15-observability.json) 包含真实采集状态、直方图计数、remote_write 结果、面板清单、规则清单、日志事件与吊销后状态，不含 Key 或 Cookie。Grafana 截图只展示前四个面板；其余面板由 provision API 与查询核对。

![真实 Senix 测试流量在 Grafana 中的表现](../../assets/readme/grafana-overview.png)

## 发现并修正的接入问题

- Alloy 的默认 10 秒抓取超时大于示例 5 秒间隔，启动会拒绝配置；显式设为 4 秒。
- VictoriaMetrics 不接受 Prometheus 的 `evaluation_interval` 和 `rule_files`；单独提供 scrape 配置，未关闭严格解析。
- Alloy 的文件 glob 需要启用内置 `file_match`；开启后真实日志查询通过。
- Docker bind mount 下 Grafana 使用 30 秒文件轮询，面板修改能被重新导入；比例面板固定 0–100% 坐标。

`promtool check config`、`promtool check rules`、Alloy `validate`、完整 Compose 展开均通过。三条告警规则已加载；本轮未模拟它们持续触发，也未配置/测试 Alertmanager 的外部通知。

## 代码回归

Rust 1.88 全工作区 79 项测试通过，含 26 项真实进程 E2E；增加日志 JSON 解析、URL/查询参数/Authorization/Cookie 不进入访问事件的检查。耗时指标检查普通请求的桶累积性、count/sum、Senix 生成的 404，以及类 gRPC 请求只在完成时进入长连接直方图且不污染普通请求统计。严格 Clippy 和格式检查通过。

依赖审计没有漏洞或已撤回锁定版本，仍保留 [AI 配置验收记录](2026-10-15-ai-configuration.md) 所列 4 条上游提示，没有新增忽略规则。

## 未验证范围

Mimir/Cortex、Thanos Receive、Grafana Cloud、Datadog、Elastic、Zabbix 提供协议路径或官方配置对照，未逐一部署。当前没有原生分布式追踪。本记录生成时未发布镜像或触发 GitHub 工作流；本机验收不能证明线上生产兼容。监控测试容器、网络、卷已清理，原有服务保留。

复现入口：[监控指南](../observability.md)、[可运行配置](../../examples/observability)、[验收脚本](../../scripts/check-observability.py)。
