# 参与开发

Senix 当前使用 Rust 1.88。本地还需要 C/C++ 编译器和 CMake，用于构建 Pingora 的原生依赖。

提交前运行：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

修改后台界面或文案后，在 `web/` 运行 `npm ci --ignore-scripts`、`npm test` 和 `npm run build`，并提交更新的 `crates/senixd/assets`。语言文案集中在 `web/src/locale.js`；测试会检查控制台源文案的英文覆盖和占位符，运行时保留用户输入及服务端原始证据。界面验证还应检查切换语言不丢草稿、不触发后台请求，以及桌面和窄屏布局。

修改依赖时还需运行 `cargo audit --deny yanked`，以及在 `web/` 中执行 `npm audit --registry=https://registry.npmjs.org --audit-level=moderate`。审计工具可用 `cargo +stable install cargo-audit --locked --version 0.22.2` 安装；项目本身仍以 Rust 1.88 验证。Cargo 按工作区的 Rust 1.88 要求选择兼容版本；锁文件与安全审计均通过后再提交。RustSec 的未维护或 unsound 提示仍需阅读，不能把退出状态为零解释为所有依赖都已无风险。

当前 Pingora 0.8.1 仍传递引入 `daemonize`、`derivative`、`unsafe-libyaml` 的未维护提示，以及 `lru 0.16.4` 的 `RUSTSEC-2026-0253` 提示。后者涉及 key 的析构触发 panic 后的内存安全；不添加忽略规则，也不把旧版本伪装为已修复。升级到包含修复的正式上游版本时应重新审计。

新功能应从已经确认的公开 Module 或系统 Seam 编写失败测试，再实现行为。不要通过测试访问内部集合或锁。模块边界记录在 [ADR-0001](docs/adr/0001-core-seams.md)。

Pull Request 需要说明改动目的、验证命令和仍未覆盖的真实环境验收。安全问题不要发公开 Issue，请使用 GitHub Security Advisory 私下报告。

修改代理热路径时，可用 [基准脚本](benchmarks/README.md) 建立前后基线，保留原始数据、机器条件与重复波动；不要从功能测试推导吞吐量。监控配置使用 [只读验收脚本](scripts/check-observability.py) 检查真实采集；示例的凭据文件只放在忽略的测试目录或外部 Secret 中。

修改 Docker Demo、引导或交付脚本后，使用 [Demo 验收脚本](scripts/check-demo.py) 检查真实启动和重启保留：先拉取 Compose 镜像，再运行 `python3 scripts/check-demo.py`。检查源码镜像时先构建，再通过 `--image senix:local` 指定；不能把发布镜像的结果当作源码镜像已验收。命令和资源边界见 [README](README.md#验证)。

Linux 发布包与 Docker 镜像共享 Bookworm 构建阶段，CI 在 glibc 2.34 的 Rocky Linux 9 中执行二进制，防止宿主系统抬高运行基线。可导出当前 Docker 平台的 Linux 二进制并检查：

```bash
docker build --target binary --output type=local,dest=target/release-portable .
docker run --rm --network none \
  --mount "type=bind,src=${PWD}/target/release-portable/senixd,dst=/senixd,readonly" \
  --entrypoint /senixd rockylinux:9-minimal --version
```

产物验证包括解包后的文件与运行环境；编译成功和 `--version` 通过只分别证明构建与基本加载。完整代理、MCP、批准与重启流程的验证范围见 [发布包兼容性记录](docs/validation/2026-10-15-portable-release.md)。
