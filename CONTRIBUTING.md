# 参与开发

Senix 当前使用 Rust 1.88。本地还需要 C/C++ 编译器和 CMake，用于构建 Pingora 的原生依赖。

提交前运行：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

修改依赖时还需运行 `cargo audit --deny yanked`，以及在 `web/` 中执行 `npm audit --registry=https://registry.npmjs.org --audit-level=moderate`。审计工具可用 `cargo +stable install cargo-audit --locked --version 0.22.2` 安装；项目本身仍以 Rust 1.88 验证。Cargo 按工作区的 Rust 1.88 要求选择兼容版本；锁文件与安全审计均通过后再提交。RustSec 的未维护或 unsound 提示仍需阅读，不能把退出状态为零解释为所有依赖都已无风险。

当前 Pingora 0.8.1 仍传递引入 `daemonize`、`derivative`、`unsafe-libyaml` 的未维护提示，以及 `lru 0.16.4` 的 `RUSTSEC-2026-0253` 提示。后者涉及 key 的析构触发 panic 后的内存安全；不添加忽略规则，也不把旧版本伪装为已修复。升级到包含修复的正式上游版本时应重新审计。

新功能应从已经确认的公开 Module 或系统 Seam 编写失败测试，再实现行为。不要通过测试访问内部集合或锁。模块边界记录在 [ADR-0001](docs/adr/0001-core-seams.md)。

Pull Request 需要说明改动目的、验证命令和仍未覆盖的真实环境验收。安全问题不要发公开 Issue，请使用 GitHub Security Advisory 私下报告。

修改代理热路径时，可用 [基准脚本](benchmarks/README.md) 建立前后基线，保留原始数据、机器条件与重复波动；不要从功能测试推导吞吐量。监控配置使用 [只读验收脚本](scripts/check-observability.py) 检查真实采集；示例的凭据文件只放在忽略的测试目录或外部 Secret 中。
