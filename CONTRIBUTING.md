# 参与开发

Senix 当前使用 Rust 1.88。本地还需要 C/C++ 编译器和 CMake，用于构建 Pingora 的原生依赖。

提交前运行：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

新功能应从已经确认的公开 Module 或系统 Seam 编写失败测试，再实现行为。不要通过测试访问内部集合或锁。模块边界记录在 [ADR-0001](docs/adr/0001-core-seams.md)。

Pull Request 需要说明改动目的、验证命令和仍未覆盖的真实环境验收。安全问题不要发公开 Issue，请使用 GitHub Security Advisory 私下报告。
