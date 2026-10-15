# 发布包运行兼容性验收

日期：2026-10-15。范围：基于 `e1a6313` 的 Linux 打包修复；未发布 Release 或镜像。

## 发现与复现

[关闭发布的四平台打包运行](https://github.com/ALVIN-YANG/senix/actions/runs/38714431509) 在干净提交 `e1a6313` 完成 Linux/macOS × x86_64/ARM64 编译和归档，镜像与 GitHub Release 发布均跳过。但下载 Linux ARM64 产物后，在 Debian 12（glibc 2.36）执行 `--version` 就失败：

```text
/senixd: /lib/aarch64-linux-gnu/libc.so.6: version `GLIBC_2.38' not found (required by /senixd)
```

动态符号表确认导入了 `__isoc23_strtol@GLIBC_2.38`。在 Rocky Linux 9（glibc 2.34）也失败；同一业务代码的 Bookworm 构建可执行。构建宿主系统的 glibc 进入了 Linux 发布包的实际运行要求。

进一步核对已发布 v0.3.1：两个 Linux 归档都导入同一 2.38 符号，归档 SHA-256 与 GitHub Release 资产摘要一致。ARM64 包在本机原生 ARM64 容器的 glibc 2.34 环境确实无法启动；x86_64 包本次只核对符号表，没有在 ARM64 宿主机上假称原生执行。旧 README 所述最低 2.34 不符合这些现有发布包，已更正为实际的 2.38，并为 2.34–2.37 用户指向 Docker Demo。旧 Release 资产未替换。

## 修复

Linux 归档现在复用现有 Dockerfile 的 Bookworm/Rust 1.88 构建阶段，以 `binary` 阶段导出独立 `senixd`。macOS 继续本机构建。每个 Linux 架构在归档前，都必须在 glibc 2.34 的 Rocky Linux 9 容器里实际执行 `--version`；失败会阻止该平台产物通过。

这个检查保留既有 2.34 兼容目标。现有 Docker 默认运行阶段、非 root 用户和启动参数保持原行为，业务代码与依赖未改。

## 本地验收结果

| 对象 | 已执行的检查 |
| --- | --- |
| 修复后的 Linux ARM64 导出文件 | 所需最高 GLIBC 符号为 2.34；在 glibc 2.34 和 Debian 12 的 2.36 均能执行 `--version` |
| 通过现有脚本归档、再解包的 ARM64 文件 | 与导出文件 SHA-256 相同，解包后在上述两个环境都可执行 |
| 新 Linux ARM64 文件，Debian 12/Python 3.12 容器 | 完整 MCP 读取、带版本规划、未批准拒绝、测试 Owner 批准、应用 v2、重复应用、新旧/无关域名真实请求、重启后原账号/Key/精确配置保留通过 |
| 四平台运行中的 macOS ARM64 归档 | 在 macOS 26.5.2 ARM64 上完成相同的完整 MCP 与重启流程；依赖仅为系统框架和 `/usr/lib` 库 |
| Rust 1.88 | 81 项通过，含 27 项真实进程 E2E；格式、严格 Clippy 通过 |
| 交付文件 | Dockerfile 检查无警告，Release YAML 和打包脚本语法检查通过 |

Linux 完整流程位于无外网、无宿主机发布端口的独立容器；临时 HTTP 后端和测试 Owner 均由验收创建并清理。测试没有声称使用真实模型客户端。最低 glibc 检查证明基本加载，完整功能流程另在 Debian 12 与 macOS 上验收；不将一种检查替代另一种。

后续对干净提交 `e552e60` 运行了 [修复后的四平台工作流](https://github.com/ALVIN-YANG/senix/actions/runs/38717842951)，四个平台打包全部通过。Linux x86_64 与 ARM64 分别在对应架构 runner 上通过 glibc 2.34 的实际运行检查。再次下载、解包两份 Linux CI 归档，确认 ELF 架构正确，最高 GLIBC 符号需求均为 2.34；[CI 结果](portable-release/2026-10-15/ci-result.json) 和 [归档/二进制 SHA-256](portable-release/2026-10-15/ci-linux-artifacts.json) 已保留。镜像发布与 GitHub Release 发布均为 skipped。

## 原始证据

[变更前 ARM64 符号与摘要](portable-release/2026-10-15/linux-arm64-before.json) · [Debian 12 启动失败](portable-release/2026-10-15/debian12-before.json) · [glibc 2.34 对照](portable-release/2026-10-15/glibc234-before.json) · [发布版 ARM64](portable-release/2026-10-15/published-arm64.json) · [发布版 x86_64](portable-release/2026-10-15/published-x86_64.json)

[修复后导出文件](portable-release/2026-10-15/linux-arm64-after.json) · [归档后检查](portable-release/2026-10-15/linux-arm64-archive-after.json) · [Linux MCP](portable-release/2026-10-15/linux-arm64-mcp.json) · [macOS MCP](portable-release/2026-10-15/macos-arm64-mcp.json) · [macOS 动态库](portable-release/2026-10-15/macos-libraries.txt) · [镜像身份](portable-release/2026-10-15/images.json) · [构建日志、Rust 输出和前后脚本](portable-release/2026-10-15/logs-and-fixtures.zip) · [SHA-256](portable-release/2026-10-15/SHA256SUMS)

运行环境均以 JSON 内镜像摘要和平台为准。公开文件只将仓库绝对路径替换为 `<REPO>`，测试凭据没有写入输出。验证归档以二进制摘要区分，临时包名与 `senixd 0.3.1` 版本输出都不代表已经发布新版本。
