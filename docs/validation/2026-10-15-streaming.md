# SSE、WebSocket 与摘流验收

日期：2026-10-15。基线提交：`f349db494fe965dcaac03e5e22a90505476e3b95`。验收对象为其后的当前源码，使用本机真实 `senixd` 进程、隔离数据库与两个 TCP 后端。本轮补充测试和证据，代理实现没有变更。

## 验证结果

| 协议 | 实际检查 |
| --- | --- |
| SSE | 后端返回 `text/event-stream` 和 HTTP/1.1 chunked body。客户端先收到包含中文的第一段完整事件，测试才允许后端生成第二段；因此整段缓存到响应结束会使测试失败。请求没有 SSE 提示头，长连接由响应类型识别。 |
| SSE 摘流 | 第一段到达后摘除 A，操作报告 `DRAINING`，普通在途数为 0、长连接为 1。三个新请求都由 B 响应，A 的原连接继续交付第二段事件并正常结束，操作转为 `DRAINED`，两类在途数归零。 |
| WebSocket | 验证真实 HTTP/1.1 Upgrade、101 响应与握手摘要。客户端帧使用随机掩码，服务端帧不带掩码；逐字节检查摘流前后的中英文双向文本。 |
| WebSocket 摘流超时 | A 摘流进入 `DRAIN_TIMEOUT` 后，三个新请求都到 B，原 WebSocket 仍能发送并收到第二条消息。随后双方交换状态码 1000 的关闭帧；长连接计数归零，超时操作保留原终态。 |
| 指标 | 两项测试分别检查：连接存续期间，长连接耗时直方图完成次数为 0；正常结束后为 1。读取操作状态确认在途租约已释放。 |

SSE 格式依据 [WHATWG Server-sent events](https://html.spec.whatwg.org/multipage/server-sent-events.html)；WebSocket 握手、掩码与关闭帧依据 [RFC 6455](https://www.rfc-editor.org/rfc/rfc6455.html)。测试只实现所需的短文本帧与关闭帧，没有引入协议客户端运行依赖。

## 复现与检查

环境：macOS 26.5.2 ARM64，Rust 1.88.0。与项目其他进程 E2E 一样，需要本机允许监听 loopback；依赖构建要求见 [贡献指南](../../CONTRIBUTING.md)。

```sh
cargo +1.88 test -p senixd --test gateway_e2e sse_delivers_events --locked
cargo +1.88 test -p senixd --test gateway_e2e websocket_remains --locked
cargo +1.88 test --workspace --locked
cargo +1.88 fmt --all -- --check
cargo +1.88 clippy --workspace --all-targets --locked -- -D warnings
```

全工作区 84 项 Rust 测试通过，含 30 项真实进程 E2E；格式、严格 Clippy 和 3 项前端文案测试通过。测试已纳入现有 Linux CI 的全工作区命令。流式 socket 的读写等待限制为 5 秒，SSE 生成第二段的等待限制为 10 秒；正常路径通过事件握手协调，不靠固定延迟判断是否逐段交付。

[检查摘要与测试源摘要](streaming/2026-10-15/checks.json) · [运行日志](streaming/2026-10-15/logs.zip) · [SHA-256](streaming/2026-10-15/SHA256SUMS)。日志不含测试 Key、Owner 密码或数据库。

## 证据边界

这是本机功能回归，不是吞吐、首段延迟、并发连接容量或长期稳定性基准。每项测试只有一条流和三个摘流后的普通请求，未覆盖慢消费者、客户端重连、WebSocket 分片/压缩、TLS 上的长流或网关进程退出。本记录生成时，gRPC 用例只验证请求头分类，不计为真实 gRPC 协议测试；后续 TLS/HTTP2 消息与 trailers 验收见 [独立记录](2026-10-15-grpc.md)。

性能图仍固定在各自原始提交与环境。流式及故障场景的独立负载模型见 [后续证据缺口](../ai-native-assessment.md)，短请求基准范围见 [基准说明](../../benchmarks/README.md)。
