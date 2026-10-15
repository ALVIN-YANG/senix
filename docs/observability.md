# 接入已有监控栈

Senix 使用两条开放通路：管理端口的 Prometheus 0.0.4 指标，以及可选的 JSON 行日志。网关不依赖采集器运行。监控系统、存储、告警通知和宿主机指标仍由外部工具负责。

请求耗时直方图和 JSON 日志从 `v0.4.0` 开始提供；旧 `v0.3.1` 包不包含这些能力。

`v0.4.0` 已发布镜像的真实采集结果见 [发布验收](validation/2026-10-15-release-v0.4.0.md)。执行验收时请使用[主分支脚本](https://github.com/ALVIN-YANG/senix/blob/main/scripts/check-observability.py)；标签内旧脚本可能因短流量早于采样或采集器冷启动连接关闭而误报，主分支已修复。

## 本机启动 Prometheus + Grafana

需要 Docker Compose v2；先按 [AI 配置指南](ai-configuration.md) 使用 `0.4.0` 镜像启动 Demo。需要本机有 bash。先在管理后台创建一把全局范围、仅含 `metrics.read` 的 Key。该 Key 只能读取指标，不能读取配置或执行变更。

从仓库根目录执行，将一次性展示的 Key 粘贴到终端提示中：

```bash
install -d -m 700 target/observability-secrets
# 可以从 bash 或 zsh 调用；输入不回显，不把 Key 写进命令历史。
bash -c 'set -e; read -r -s -p "Metrics key: " key; printf "%s" "$key" > target/observability-secrets/metrics-token; echo'
chmod 644 target/observability-secrets/metrics-token
export SENIX_METRICS_KEY_FILE="$PWD/target/observability-secrets/metrics-token"
export SENIX_DOCKER_NETWORK=demo_senix-demo

docker compose -p senix-monitoring -f examples/observability/compose.yml up -d prometheus grafana
```

宿主机目录为 `0700`，文件为 `0644`，让容器中的非 root 采集器能读取显式挂载的文件；目录禁止其他本机账号遍历。密钥文件只读挂载，不写入镜像、Git 或 Grafana。生产环境使用自己的 Secret 管理与轮换方案。

Demo 默认 Compose 项目名为 `demo`，网络为 `demo_senix-demo`。如果启动 Demo 时设置了 `COMPOSE_PROJECT_NAME`，将 `SENIX_DOCKER_NETWORK` 改成实际网络名；采集器必须能解析 `senix` 并访问 `9080`。

- Grafana：[本机面板](http://127.0.0.1:13000/d/senix-overview/senix-overview)。数据源与 8 个面板会自动导入，包含 RPS、5xx、p95、在途数、快照版本、证书风险、代理错误和实例状态。
- Prometheus：[本机 Targets](http://127.0.0.1:19090/targets)。应看到 `senix` 为 UP；至少等待两次采集后查看速率。
- 产生测试流量：`curl -H 'Host: demo.senix.local' http://127.0.0.1:8080/`。如果更改过 Demo 域名或端口，使用实际值。
- 停止：`docker compose -p senix-monitoring -f examples/observability/compose.yml down`。先停止采集器再删除 Key 文件；也可在后台吊销 Key 验证采集失败。

这套 Grafana 只对本机开放匿名 Viewer，未创建管理员；适合本地试用。生产接入已有 Grafana 的登录、TLS、持久化、备份和通知配置。Prometheus 内含三条告警规则，但示例没有配置 Alertmanager，不会向外发送通知。

## 兼容矩阵与验证级别

| 监控系统 | 接入方法 | 验证级别 |
| --- | --- | --- |
| Prometheus + Grafana | 带 Bearer 的 `/metrics`；预置面板和规则 | 本机真实采集、PromQL、Grafana 数据源与面板验证 |
| VictoriaMetrics / vmagent | Prometheus scrape；或 remote_write | 单机 VictoriaMetrics 的真实采集验证；未部署集群版 |
| OpenTelemetry Collector | Prometheus receiver → metrics pipeline → exporter | Collector 真实采集并重新导出验证；可换成自己的 OTLP 后端 |
| Grafana Alloy + Loki | Alloy 抓指标后 remote_write；JSON 文件日志 → Loki | 真实 remote_write 与日志查询通过；不要求 Docker socket 权限 |
| Mimir / Cortex / Thanos Receive / Grafana Cloud | 现有 Prometheus/Alloy remote_write 到对应端点 | 配置示例、协议路径；未部署每个服务或云账号 |
| Datadog | Agent OpenMetrics 集成，文件读取 Bearer | 官方配置对照，未使用 Datadog 账号验收 |
| Elastic / ELK | Metricbeat Prometheus collector；Filebeat ndjson 日志 | 官方配置对照，未部署 Elastic 集群验收 |
| Zabbix | HTTP Agent 加 Authorization 头，依赖项 Prometheus pattern | 官方文档确认接入路径，未部署 Zabbix 验收 |
| 其他接受 Prometheus 或 OTLP 指标的平台 | 直接抓取，或通过 OTel Collector 转换 | 需按实际 exporter、认证和平台限制验收 |

不能由“支持 OTLP 指标出口”推导出 Senix 已支持分布式追踪。当前没有原生 trace/span、OTLP trace exporter 或 exemplar，不能宣称已完整接入 Tempo/Jaeger/APM。业务应用自己的 trace 上下文是否透传也未在本轮作为契约验收。

## 现有采集器与云端后端

复用 [prometheus.yml](../examples/observability/prometheus.yml) 的 `scrape_configs`，把 `senix:9080` 改成从采集器可达的私网地址，并挂载凭据文件。不要公开管理端口；跨主机通过受信 HTTPS 入口传输 Key。`/healthz` 公开，只表示管理进程存活；`up` 表示采集成功，两者均不表示业务路由可用。

示例配置：

- [OTel Collector](../examples/observability/otel-collector.yml)：Prometheus receiver，batch processor，本地 Prometheus exporter。生产可用自己的 OTLP exporter 转发到已有平台；需要相应 exporter 的 TLS、认证和资源标签设置。
- [Alloy](../examples/observability/config.alloy)：指标发送到本机 VictoriaMetrics；日志发送到 Loki。
- [remote_write](../examples/observability/remote-write.yml)：Mimir/Cortex、Thanos Receive、VictoriaMetrics 的端点形式；认证与租户需要填写。Senix 采集 Key 和远端写入 Key 是两把不同凭据。
- [Datadog](../examples/observability/datadog.yml)、[Metricbeat](../examples/observability/metricbeat.yml)、[Filebeat](../examples/observability/filebeat.yml)：合并到现有 Agent 配置，单独设置输出和凭据。
- Zabbix：HTTP Agent 请求 `http://<private-host>:9080/metrics`，添加 `Authorization: Bearer {$SENIX_METRICS_KEY}`；宏使用 Secret 类型。主项取 Body、要求 200，依赖项用 Prometheus pattern，例如 `senix_config_snapshot_version`。

可选的本机采集器验证栈：

```bash
# 在前面环境变量的基础上启用 Collector、VictoriaMetrics、Alloy、Loki。
export SENIX_LOG_DIR="$PWD/target/observability-logs"
mkdir -p "$SENIX_LOG_DIR"
docker compose -p senix-monitoring -f examples/observability/compose.yml --profile collectors up -d
```

附加端口也只绑定本机：Collector `18889`、VictoriaMetrics `18428`、Loki `13100`。不要同时向同一后端重复发送相同 job/instance 的时间序列；示例中的 Alloy 使用独立的 `senix_alloy` job 来展示第二条路径。

## 指标含义

| 指标 | 含义与使用边界 |
| --- | --- |
| `senix_proxy_requests_total` | 已进入代理请求过滤器的请求，含路由拒绝；不含 TLS 握手失败 |
| `senix_proxy_responses_total{class}` | 实际产生响应的 HTTP 状态类别，包括 Senix 生成的错误；不等于所有请求都产生响应 |
| `senix_proxy_errors_total` | 以 Pingora 错误结束的请求；与 5xx 含义不同，不应相加当错误率 |
| `senix_proxy_request_duration_seconds{kind}` | 从请求过滤器到响应体结束的生命周期直方图，含后端耗时；不含入口 TCP/TLS 建连 |
| `senix_instance_in_flight{instance_id,kind}` | 已分配到实例、尚未结束的普通请求或长连接；不是 TCP 连接数 |
| `senix_instance_info` | 当前流量、健康状态与代次；历史状态由监控存储保留 |
| `senix_config_snapshot_version` | 当前发布版本 |
| `senix_certificate_*` / `senix_certificates_*` | 托管证书启用、过期、30 天到期风险；主机 TLS 探测仍可使用外部 blackbox exporter |

计数器随进程重启归零，使用 `rate`/`increase`。直方图固定桶为 0.5ms 至 5s 加 `+Inf`；超过最大有限桶时无法准确估计高分位。WebSocket、SSE、gRPC 被分类为 `long_lived`，只在流结束时记录其完整生命周期，普通请求 p95 应过滤 `kind="ordinary"`。没有按 URL/Host/Key 建标签；实例 ID 和代次变化也会产生新时间序列，应控制实例数量与更新频率。

```promql
sum(rate(senix_proxy_requests_total[5m]))
histogram_quantile(0.95, sum by (le) (rate(senix_proxy_request_duration_seconds_bucket{kind="ordinary"}[5m])))
sum(rate(senix_proxy_responses_total{class="5xx"}[5m])) / clamp_min(sum(rate(senix_proxy_responses_total[5m])), 0.001)
sum by (kind) (senix_instance_in_flight)
```

业务可用性仍需用 blackbox exporter 或外部探针对真实 Host/路径发请求；CPU、RSS、磁盘和容器指标使用 node_exporter/cAdvisor 或现有主机 Agent，Senix 不重复实现这些采集器。

## JSON 日志

默认保持文本格式、默认不记录逐请求日志。`v0.4.0` 可显式开启：

```bash
RUST_LOG=senixd=info,senix_pingora::access=debug senixd \
  --log-format json \
  --listen 127.0.0.1:8080 --admin-listen 127.0.0.1:9080 \
  --db /var/lib/senix/senix.db --config /etc/senix/gateway.json \
  >> /var/log/senix/access.jsonl
```

父目录与轮转由运行环境管理。容器可把 stdout 交给现有日志驱动或 Agent；本机示例也可将 `docker logs --follow <senix-container>` 输出到 `SENIX_LOG_DIR` 中的 `.jsonl` 文件。默认文本日志不能直接交给 ndjson parser。`--log-format json` 只控制 tracing 日志；CLI 子命令的 JSON/提示输出不应混入运行时日志文件。

逐请求事件的 `fields` 仅含 `message=proxy_request_completed`、状态码、持续秒数、长连接分类、实例 ID 和错误布尔值。不记录 URL、查询参数、Authorization、Cookie、请求/响应体或错误全文。不要用全局 `RUST_LOG=debug/trace` 替代上面的精确 target；第三方库的调试输出不在此日志契约内。开启逐请求日志有额外 I/O 成本，本轮性能基线关闭该选项。

## 配置依据

这些文档说明协议接入方式，不替代本项目的真实运行验证：[Prometheus 认证与 remote_write](https://prometheus.io/docs/prometheus/latest/configuration/configuration/)、[Grafana provisioning](https://grafana.com/docs/grafana/latest/administration/provisioning/)、[OTel Prometheus receiver](https://github.com/open-telemetry/opentelemetry-collector-contrib/tree/main/receiver/prometheusreceiver)、[Alloy scrape](https://grafana.com/docs/alloy/latest/reference/components/prometheus/prometheus.scrape/)、[Datadog OpenMetrics 配置](https://github.com/DataDog/integrations-core/blob/master/openmetrics/datadog_checks/openmetrics/data/conf.yaml.example)、[Metricbeat Prometheus](https://www.elastic.co/guide/en/beats/metricbeat/current/metricbeat-module-prometheus.html)、[Filebeat ndjson](https://www.elastic.co/guide/en/beats/filebeat/current/filebeat-input-filestream.html#filestream-ndjson-parser)、[Zabbix Prometheus 检查](https://www.zabbix.com/documentation/7.0/en/manual/config/items/itemtypes/http/prometheus)。

本机栈启动后，可运行只读验收脚本。它会产生 20 次真实代理请求并检查认证、PromQL、面板、规则；`--collectors` 还检查 VictoriaMetrics、OTel、Alloy 和 Loki。Loki 检查需要前面说明的真实 JSON 日志采集已启动。

```bash
python3 scripts/check-observability.py --key-file "$SENIX_METRICS_KEY_FILE"
# 若使用了附加采集器，并已向日志目录持续写入 Senix JSON 日志：
python3 scripts/check-observability.py --key-file "$SENIX_METRICS_KEY_FILE" --collectors
```

端口/域名被覆盖时使用 `--admin`、`--proxy`、`--host` 等参数。完整证据见 [本轮监控验收记录](validation/2026-10-15-observability.md)。
