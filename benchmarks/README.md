# 可复现的代理基准

这套基准衡量固定 1 KiB 响应、Keep-Alive 和闭环并发下的端到端成本。它不证明生产容量，也不用于声称 Senix 比 Nginx、Envoy 或其他网关更快。

## 运行

需要 Python 3、Docker（Linux 容器、cgroup v2）、openssl，以及至少约 4 GiB 可用内存。先停止本机其他负载。所有负载容器都运行在同一个 Docker bridge 网络，分别限额 2 CPU / 1 GiB；没有发布任何宿主机端口。脚本仅创建带随机前缀的测试资源并在结束或失败后清理。

```bash
docker build -t senix:observability .
docker pull grafana/k6:2.3.0
docker pull node:24-bookworm-slim
python3 benchmarks/run.py --image senix:observability --output target/benchmarks-local
```

默认比较：

| 场景 | 入口 | 路由数量 | 上游 |
| --- | --- | --- | --- |
| `direct-http` | Node HTTP/1.1 | 无网关 | 固定 1 KiB 的单 Node worker |
| `senix-http` | Senix HTTP/1.1 | 1 | 同一 Node 后端 |
| `senix-https` | Senix HTTPS，客户端支持 H2 | 1 | 同一 HTTP 后端 |
| `senix-http-100-routes` | Senix HTTP/1.1 | 100 | 命中同一精确域名和 `/` 路径 |

每场景分别使用 1、32、128 个 VU，各预热 3 秒，再独立测量 10 秒、重复 3 次。重复运行之间复用网关进程；100 路由场景重建进程和数据库。脚本中没有强制调优 Pingora worker 数，使用本项目默认配置。没有日志逐请求写入、监控采集或持续配置发布。

快速检查脚本能运行：`python3 benchmarks/run.py --vus 1 --repeats 1 --duration 1s --warmup 1s --output target/benchmarks-smoke`。短运行只用于检查，不作为性能证据。输出目录不能复用已有结果，避免混入其他运行数据。

## 数据与解释

每次运行保存 k6 `summary.json`、负载器日志，以及测量前后的 cgroup CPU 计数和 `/proc/1/status` 中的进程 RSS/HWM。总目录保存逐次结果、三次中位数及 RPS 最小/最大值、镜像 ID/摘要、Docker 虚拟机 CPU/内存/内核、源文件 SHA-256、Git HEAD 与工作区是否有改动。提供的 `--image` 必须由对应源码构建，脚本不会证明外部镜像与工作区一致。镜像标签可能变化，重跑正式测量时使用记录中的 digest 或镜像 ID。

- RPS 来自 k6 完成请求数/测量时间；p50/p95/p99 来自 `http_req_duration`，单位毫秒。它包含请求发送、等待后端和接收响应，不包含 DNS、TCP 和 TLS 建连；HTTPS 用复用连接，不能解释为 TLS 握手吞吐量。
- `checks` 要求状态 200 且 Content-Length=1024；`http_req_failed` 必须为零，否则脚本失败并保留现场结果。单次函数不读取响应体内容，负载固定由仓库中的 backend 实现产生。
- HTTPS 使用仅供测试的自签名证书，并在 k6 中关闭验证；这不是生产配置建议。`http2_responses` 记录实际 H2 响应数，不能只按 URL 推测协议。
- 三次 p95 的中位数是“单次 p95 的中位数”，不是合并所有请求后重新计算的总体 p95。汇总请求数可跨三次求和。
- CPU 由容器 cgroup `usage_usec` 增量/实际窗口时长计算，以单核心 100% 计。窗口包含 k6 容器启动和退出，可能比实际负载时长更长。RSS 是 Senix PID 1 的测量结束样本；HWM 是该进程生命周期的峰值，包含预热和之前的测试，不是每次运行独立的峰值。
- Node 后端、k6、网关共享同一台 Docker VM；Node 单 worker 或负载器都可能成为瓶颈。1/32/128 VU 不是 QPS 目标，也不是最大并发连接数。没有证明最大可承载流量或饱和点。
- 100 路由只验证本次精确域名命中；没有覆盖大量通配路径、多后端、长流、大请求体、网络故障或生产持续负载。

正式结果和图表会链接到独立的验证记录，不能把本机单次结果推广成所有硬件或发布包的性能保证。

## 生成可公开核对的报告

原始测量结束后，用标准库脚本归档各次结果、采样和校验和；它只移除元数据中的本机绝对输出路径。`--chart` 选项使用 matplotlib 生成图表，不影响或重新运行测量。

```bash
# 不画图时不需要额外 Python 包。
python3 benchmarks/report.py --input target/benchmarks-local --output target/benchmark-report
# 可选绘图（本轮使用 Python 3.14 + matplotlib 3.11.2）。
python3 -m pip install matplotlib==3.11.2
python3 benchmarks/report.py --input target/benchmarks-local --output target/benchmark-report-with-chart \
  --chart target/benchmark-32vus.svg --vus 32
```

汇总中提供 Senix 容器 CPU 窗口均值的三次中位数、结束时 RSS 最大样本，以及进程 HWM。它们不能替代精确进程剖析；直连场景下网关处于空闲，不应将该资源数与代理负载比较。

在其他环境测量后绘图，用 `--environment-label` 填写准确的环境说明，例如 `--environment-label 'GitHub Linux runner'`。此选项只更改图注，不改变测量数据；环境与镜像身份仍以元数据为准。

仓库另含仅 `workflow_dispatch` 触发的 [benchmark 工作流](../.github/workflows/benchmark.yml)，会构建镜像并保存原始结果，即使失败也尝试上传结果。本轮验证在本机执行，没有触发远端工作流，也没有用共享 CI runner 的波动数据设置硬性能门槛。
