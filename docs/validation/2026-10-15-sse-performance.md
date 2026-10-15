# Linux SSE 分段交付基线

测量时间：2026-10-15 15:38:02–15:50:18 UTC。源码为干净提交 `dba2841a57ec9da51ad9dbc888f5f9c1103c2be4`，属于尚未发布的主分支快照，不将其称为已发布 `v0.4.0` 或未来版本的性能结果。[实际工作流](https://github.com/ALVIN-YANG/senix/actions/runs/38761262564) 成功完成全部矩阵。

## 结果与模型

27 次测量完成 **6,498 条完整流、649,800 条事件**，其中 **4,263 条流、426,300 条事件经过 Senix**。客户端流失败为 0，逐事件顺序、中文内容、字节数和结束边界校验均通过；后端完成数与客户端完成数一致，没有中断流。HTTPS 场景的 1,890 条完成流均使用实际协商的 HTTP/2。

每条流是合成的定速输出：100 条预先构造的事件，每条 `data:` 为 512 字节 JSON，包含序号、中文与填充；连同 SSE 格式，每条响应体为 52,690 字节。收到请求后立即写入首条，之后按 50 ms 定时器调度，实际间隔还受调度和背压影响。它没有调用模型，不代表真实模型推理速度。

三种路径分别测试 1、32、128 个客户端 VU，每组预热窗口 3 秒，再测量 20 秒窗口、重复三次。窗口结束停止发起新请求，已有流自然完成；实际负载窗口为 20.025–25.034 秒。完整流数是样本量，定速周期与窗口边界会影响最后一轮启动数量，不把各组流数当作吞吐优劣或最大容量。

下表的 p95 是三次单独 p95 的中位数；完整流数是三次之和，活动流峰值与结束 RSS 是三次最大样本。直连时网关空闲，因此不展示其 RSS 作为代理负载比较。

| 场景 | VU | 首事件 p95 ms | 事件间隔 p95 ms | 后端活动流峰值 | 完整流数 | 网关结束 RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Direct HTTP/1.1 | 1 | 9.867 | 50.472 | 1 | 15 | — |
| Direct HTTP/1.1 | 32 | 14.582 | 51.127 | 32 | 402 | — |
| Direct HTTP/1.1 | 128 | 50.134 | 50.863 | 128 | 1,818 | — |
| Senix HTTP/1.1 | 1 | 9.641 | 50.305 | 1 | 15 | 21.6 |
| Senix HTTP/1.1 | 32 | 15.312 | 51.031 | 32 | 480 | 26.8 |
| Senix HTTP/1.1 | 128 | 57.972 | 51.042 | 128 | 1,878 | 41.3 |
| Senix TLS/H2 (one connection) | 1 | 13.237 | 50.391 | 1 | 15 | 42.5 |
| Senix TLS/H2 (one connection) | 32 | 13.920 | 51.245 | 32 | 479 | 42.6 |
| Senix TLS/H2 (one connection) | 128 | 5000.579 | 51.323 | 100 | 1,396 | 42.6 |

![32 VU 下的首个完整 SSE 事件延迟；横线是三次 p95 的最小–最大值](../../assets/readme/sse-first-event.png)

图选用 32 VU；完整矩阵保留在上表和 [逐次结果](sse-performance/2026-10-15/runs.json)，包括 128 VU 下的排队结果。首事件时间包含初始 DNS/TCP/TLS 等待、客户端调度与解析、后端行为，不能把两组 p95 相减当作网关内部开销。图中的横线不是置信区间。

## 单连接 HTTP/2 的 128 VU 结果

该负载器用一条 TLS/HTTP2 连接复用请求。运行时记录到服务端通告的 `max_concurrent_streams=100`；128 VU 时后端活动流峰值为 100，首事件 p95 中位数为 **5,000.579 ms**，而相邻事件间隔 p95 为 **51.323 ms**。

结合连接设置、活动流数和延迟分布，判断此处存在流槽位排队；本轮没有独立剖析客户端等待与代理执行时间。该限制按连接生效，不能由此推导网关的总并发上限。它也不构成相同活动流数下 HTTP/1.1 与 HTTP/2 成本的对比。需要更高客户端并发时，应单独评估连接池和目标流量。本轮保留网关默认的 HTTP/2 设置。

## 环境与资源含义

- GitHub 托管 Linux AMD64 runner：Ubuntu 24.04.5 LTS，4 CPU、15.61 GiB RAM，Docker 28.0.4，内核 `6.17.0-1022-azure`。
- 网关、单 worker Node 后端、Node 负载器分别限额 2 CPU / 1 GiB，三者共享 runner。Node 为 v24.22.0。容器之间使用专属 bridge 网络，没有发布宿主端口。
- 网关镜像由该提交在 CI 构建，镜像 ID 为 `sha256:fb292fbbfdab3bd69a9dbfa124fbd46a1b2dadf82034613020dd45e43dfaec35`。构建配方、测量脚本和实际后端镜像摘要均在 [元数据](sse-performance/2026-10-15/metadata.json) 中记录；此 CI 本地镜像没有推送到镜像仓库。
- 下游为 HTTP/1.1 或 TLS/HTTP2，上游始终是 HTTP/1.1。HTTP/1.1 使用最多 VU 条 Keep-Alive 连接，HTTP/2 使用一条共享连接。测试 TLS 使用一次性自签名证书，客户端关闭校验，不作为生产设置建议。
- 顺序固定为直连、Senix HTTP/1.1、Senix TLS/HTTP2；同一网关和后端跨 VU、预热、重复及协议场景复用。连接池和内存状态会受前序运行影响，没有随机平衡场景顺序。
- 代理负载下 RSS 最大结束样本为 **42.63 MiB**。本次 PID 1 的生命周期 HWM 也是 42.63 MiB；两者测量含义仍不同。CPU 由 cgroup 计数增量除以窗口时长计算，窗口包含负载容器启动与退出，不能当作精确的单请求 CPU 剖析。
- 没有逐请求访问日志、外部监控采集、配置发布或摘流负载。负载器、单 worker 后端和共享 runner 都可能限制结果。

## 校验与复现

需要 Python 3、Docker 的 Linux 容器与 cgroup v2、openssl，约 4 GiB 可用内存，以及镜像/依赖下载网络。先停止其他负载，再按 [基准说明](../../benchmarks/README.md#sse-长响应) 构建对应源码镜像。手动 benchmark 工作流选择 `workload=paced-sse`；默认的短请求工作流保持原模型。

```bash
docker build -t senix:benchmark .
docker pull node:24-bookworm-slim
python3 benchmarks/streaming.py --image senix:benchmark --output target/streaming-benchmark
python3 benchmarks/report.py --input target/streaming-benchmark --output target/streaming-report
```

导出器用 Python 从整数微秒样本独立重算 p50/p95/p99，再交叉核对完成流、事件、字节、实际协议、后端计数和资源。全部 27 次通过。[校验结果](sse-performance/2026-10-15/verification.json)。

负载器的正常、截断、乱序、内容损坏四项反向检查已纳入 [普通 CI](https://github.com/ALVIN-YANG/senix/actions/runs/38760912407)。导出器新增三项基于真实归档的 CLI 回归，并加入当前 CI：正常导出、改动原始样本在生成报告前被拒绝、嵌套输出目录在复制前被拒绝。同一历史输入的旧短请求资源汇总保持原数值。绘图使用 matplotlib 3.11.2，不参与测量。

下载的原始 GitHub artifact ZIP 摘要与 API 记录一致：`8fcea52fe65e4dd70612d7d057f45ad649513bbb8b4237e86de2cfe659c728d8`。公开归档只替换元数据中的绝对输出路径并增加说明；原始样本、逐次结果和其余原始文件逐字节保留。报告新增的表格和校验记录单独可核对。

[完整原始样本、预热与运行日志 ZIP](sse-performance/2026-10-15/raw-results.zip) · [汇总](sse-performance/2026-10-15/summary.json) · [交付检查](sse-performance/2026-10-15/checks.json) · [SHA-256](sse-performance/2026-10-15/SHA256SUMS)。GitHub artifact 自身有保留期限，仓库中的副本用于长期核对。

## 边界

这是共享 CI runner 上的短时、定速 SSE 基线，没有证明最大并发、饱和点、生产容量、长期稳定性或优于其他网关。没有慢消费者、网络故障、流量中改配置/摘流、WebSocket/gRPC 压测或大请求体。本轮没有重测已发布安装包与镜像，也没有由测试通过推导真实用户增长。

本基线与 [固定 1 KiB 短请求基线](2026-10-15-linux-performance.md) 的模型、源码和运行环境不同；它们不能作为优化前后对比。短请求延迟排除了建连，SSE 首事件包含初始连接等待，两者不能直接比较。
