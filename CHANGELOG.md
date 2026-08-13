# Changelog

Senix 使用[语义化版本](https://semver.org/lang/zh-CN/)。`0.x` 阶段仍可能调整接口，破坏性变化会在发布说明中明确标出。

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
