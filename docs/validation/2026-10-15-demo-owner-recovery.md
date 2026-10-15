# Demo Owner 引导失败后的恢复

日期：2026-10-15。范围：基于 `db8db32` 的启动脚本与验收增量。此修复直接用于仓库中的 Demo 脚本，已发布 0.3.1 镜像和当前源码镜像均可配合使用；没有新增 Release 或发布镜像。

## 真实复现

在隔离的全新 Compose 项目中，首次使用 `SENIX_DEMO_PASSWORD=short` 启动。引导 Credential 创建成功，Owner 密码校验失败，脚本退出 1。改成合格密码重跑，旧脚本退出 0 并显示 ready，代理也能工作，但 Owner 登录返回 `409 OWNER_ACCOUNT_NOT_INITIALIZED`。

这不是密码错误重试失败：脚本只在 Credential 本次创建成功时创建 Owner，已存在的 Credential 会让它跳过账号创建。Credential 与 Owner Account 是两个独立的持久化步骤，前一步成功不能证明后一步成功。

[原始复现](demo-owner-recovery/2026-10-15/before-original.json) 与 [新增验收命令的失败结果](demo-owner-recovery/2026-10-15/before-regression.json) 均保留。先增加 `--bootstrap-failure` 验收，再运行旧启动脚本，确认检查在真实 Owner 登录阶段失败后才修复。

## 修复与验证

启动脚本现在分别确认两个引导步骤。Credential 已存在时，仍调用现有本机 `owner bootstrap` 补齐尚未创建的账号；已创建账号由领域层的一次性约束保护，保留原密码和配置。只有账号新建成功时才输出首次密码，其他初始化错误仍会退出。

| 检查 | 实际结果 |
| --- | --- |
| 原始“短密码失败 → 改为合格密码重跑” | 修复后重跑退出 0，Owner 登录 200；[结果](demo-owner-recovery/2026-10-15/after-original.json) |
| 当前源码 ARM64 镜像，`--bootstrap-failure` | 通过账号补齐、真实代理、运行中重跑、批准应用域名变更及重启后精确 v2 配置与原账号保留；[结果](demo-owner-recovery/2026-10-15/after-source.json) |
| 发布 ARM64 镜像，组合两种故障 | `--bootstrap-failure --startup-failure` 通过；短密码失败、端口占用失败后均可恢复，密码已输出并保留，完整变更流程通过；[结果](demo-owner-recovery/2026-10-15/after-published.json) |
| 发布镜像，不注入故障的首次启动 | 完整首次、重跑、变更、重启与清理通过；[结果](demo-owner-recovery/2026-10-15/after-normal.json) |
| Rust 1.88 回归 | 81 项通过，含 27 项真实进程 E2E；格式与严格 Clippy 通过 |
| 交付检查 | Shell、Python、CI YAML 语法、README 图片与相对文件链接通过 |

沿用当前锁定镜像，无需重建业务二进制；具体镜像 ID、架构、digest、验收脚本 SHA-256、阶段结果与耗时均在 JSON 中。CI 已配置组合故障验收，输出继续按运行编号和重试次数隔离在 runner 临时目录。

复现和验收命令，均在仓库根目录执行，需要已缓存的镜像、可用的 18080/19080 端口和 Demo 子网：

```bash
python3 scripts/check-demo.py --bootstrap-failure --startup-failure
# 指定已构建的源码镜像时增加 --image senix:local。
```

验收使用测试 Owner 的真实 REST API，不代表某个模型客户端的任务成功率。所有测试资源已清理，原有服务保留；公开输出已移除密码、Key 和本机仓库绝对路径。

## 与此前未知退出的区别

此前未保留输出的偶发启动退出，在本次修复前连续 10 次普通启动、真实代理与 Owner 登录中均未复现，见 [跟进结果](demo-owner-recovery/2026-10-15/earlier-startup-followup.json)。这些成功运行不能证明那次故障已解决。本记录修复的是另行稳定复现的“两阶段引导中断后无法登录”，不能把它推定为此前未知退出的根因。

[脱敏日志、原始 Rust 输出和修复前后脚本](demo-owner-recovery/2026-10-15/logs-and-fixtures.zip) · [SHA-256 清单](demo-owner-recovery/2026-10-15/SHA256SUMS)。归档脚本用于核对当时状态，日常验收使用仓库中的 `scripts/check-demo.py`。
