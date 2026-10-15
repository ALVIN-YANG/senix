# Changelog

Senix 使用[语义化版本](https://semver.org/lang/zh-CN/)。`0.x` 阶段仍可能调整接口，破坏性变化会在发布说明中明确标出。

## [Unreleased]

### Added

- MCP `plan_change.candidate` 改为实际配置类型派生的 JSON Schema，公开路由、后端、TLS 和健康检查字段。
- 增加 GitHub Linux AMD64 runner 的 36 次基准、原始结果与校验和，中英文 README 展示该环境图表，同时保留 ARM64 本机基线。
- 增加隔离 Docker Demo 验收脚本，检查首次启动、运行中重跑、Owner 批准配置变更及重启后的配置和账号保留，失败时保存脱敏日志与退出状态。
- 增加英文 README 和完整 AI 配置操作指南，中英文页面互链，`llms.txt` 提供两种语言入口。
- MCP `get_current_config` 通过全局 `change.read` 返回配置版本与完整配置，便于 AI 保留其他路由后生成变更计划。
- 后台增加只读诊断、配置提案和配置协作 Key 模板，明确显示全局范围和各自的操作边界。
- 增加当前源码的 Docker 启动、Cursor 接入、Owner 批准及真实请求验证指南。
- 增加普通请求与长连接分离的 Prometheus 耗时直方图；网关生成的错误响应也计入状态类别。
- 增加 `--log-format json` 和显式开启的脱敏逐请求事件，便于外部日志采集。
- 增加 Prometheus/Grafana、VictoriaMetrics、OTel Collector、Alloy/Loki 可运行示例，以及其他监控系统的协议接入配置与验证级别。
- 增加固定负载、多次重复、保存原始数据的 Docker/k6 基准及手动触发工作流；README 展示真实操作与性能证据。
- 增加 `llms.txt` 文档入口与 AI 使用能力核查，明确现有保证和仍缺少的接口约束与评测能力。

### Changed

- **MCP 破坏性调整（未发布）**：`plan_change` 必填 `expected_version`，旧调用方须传入 `get_current_config` 返回的版本；版本不符返回 `CONFIG_VERSION_MISMATCH`，不创建计划，须重读并合并配置。缺参或配置结构错误由 SDK 返回工具错误。REST v1 的现有请求体及 `plan_rollback` 保持兼容。

### Fixed

- 安装器在覆盖已有程序前验证候选二进制可执行；运行环境不兼容时退出并保留旧程序，避免校验和正确但无法加载的新包损坏已有安装。
- Linux 发布包复用 Bookworm 构建环境，并在 glibc 2.34 运行验证，避免宿主 Ubuntu 的新 glibc 引入 `GLIBC_2.38` 依赖。更正已发布 v0.3.1 Linux 包的实际环境要求；旧包未被替换。
- Demo 在引导 Credential 已存在但 Owner 账号尚未创建时，重跑继续完成账号创建，避免错误密码或引导中断后显示 ready 却无法登录；已有账号、密码与配置继续保留，CI 覆盖该恢复路径。
- CI 的 Demo 验收输出移到 runner 临时目录，按运行编号与重试次数隔离，避免 Rust 缓存恢复旧输出目录后验收提前失败；失败证据上传使用同一路径。
- MCP 配置规划在发布锁内核对读取版本，避免读取后发生发布时仍创建会覆盖并发修改的旧候选计划；加入重读、保留新增路由、批准和应用的完整回归。
- Demo 的本机健康和代理探针增加连接及请求超时，避免 TCP 已连接但无 HTTP 响应时一直挂起；Owner 创建成功后立即输出首次密码，启动失败后仍可保留密码并重试。
- 更新 `h2`、`rustls`、`chacha20` 和前端传递依赖 `source-map-js`，修复已知漏洞并移除已撤回的锁定版本；前端 CI 增加官方 npm 审计。
- 默认 Demo 使用已发布的 `0.3.1` 镜像；本地构建可通过 `SENIX_DEMO_PULL=0` 使用，不强制拉取远程同名镜像。
- Demo 支持覆盖代理和管理端口，并在启动容器前校验端口范围。
- 重跑已启动的 Demo 时复用运行容器执行本机引导检查，避免临时容器与静态 IP 冲突；保留已有账号和数据。
- Cargo 更新按工作区 Rust 1.88 要求选择兼容依赖，避免自动更新引入更高编译器要求。
- RustSec 检查增加对 cargo-audit 退出状态的独立检查，已登记的漏洞和已撤回版本也会阻止检查通过。

## [0.3.1] - 2026-08-14

### Fixed

- 安装器在连接中断、超时和临时 HTTP 错误后有界重试，避免一次公网抖动直接中止安装。
- 支持通过 `SENIX_RELEASE_BASE_URL` 指向受信 HTTPS Release 镜像；HTTP 只允许测试使用的 loopback 地址。

## [0.3.0] - 2026-08-14

### Added

- 受 Bearer Key 保护的 Prometheus 指标，覆盖代理结果、配置版本、实例状态、在途请求和证书到期风险。
- SQLite 在线一致性备份、完整性校验和不覆盖现场文件的恢复命令。
- 可直接用于容器和 systemd 的 `senixd healthcheck` 进程探针。
- 上游 HTTPS、证书校验、SNI、HTTP/1.1 与 HTTP/2 ALPN，以及相同配置下的主动健康检查。
- 精确域名、单标签通配域名、边界安全的最长路径匹配和更严格的配置校验。
- 管理后台手动证书续期入口。

### Security

- Owner 登录按真实 TCP 来源限速，并限制 Argon2 并发校验。
- GitHub Actions 全部固定到完整提交，增加 RustSec 定期扫描和多生态 Dependabot 更新。
- 用隔离兼容包移除 Pingora 0.8.1 传递引入的易受攻击 `protobuf 2.28.0`。
- 丢弃配置切换期间返回的过期健康探测结果，避免旧目标状态污染新目标。

### Changed

- SQLite 启用 WAL、`synchronous=FULL`、忙等待和明确的 Schema 版本检查。
- 所有管理错误响应禁止缓存；证书列表在相同毫秒时间戳下保持稳定顺序。

[0.3.0]: https://github.com/ALVIN-YANG/senix/compare/v0.2.2...v0.3.0
[0.3.1]: https://github.com/ALVIN-YANG/senix/compare/v0.3.0...v0.3.1
