# ADR-0005：隔离 Pingora 0.8.1 的 Prometheus 安全兼容

- 状态：已接受，临时措施
- 日期：2026-08-14

## 背景

Pingora 0.8.1 的已发布 `pingora-core` 依赖 `prometheus 0.13.4`，继而固定到受 CVE-2025-53605 影响且没有 2.x 修复版的 `protobuf 2.28.0`。Pingora 主分支已将 Prometheus 升到 0.14，但当前没有包含该修复的新正式版本。Senix 不应忽略安全告警，也不应为一个传递依赖切换到未发布的 Pingora 主分支。

## 决定

工作区提供一个不发布的 `prometheus 0.13.5` 兼容包，通过 Cargo Patch 满足 Pingora 0.8.1 的版本范围，并完整重导出官方 `prometheus 0.14`。只映射 Pingora 当前使用的默认 Protobuf 功能，不复制实现，不增加自维护协议代码。

Cargo.lock 必须只包含 `protobuf >= 3.7.2`。严格 Clippy、工作区测试、真实 Pingora 代理测试和 RustSec Workflow 共同验证兼容性。

## 移除条件

Cloudflare 发布直接依赖安全版 Prometheus 的 Pingora 正式版本后，优先升级 Pingora，并删除兼容包和 `[patch.crates-io]`。不得把本措施扩展成通用的依赖版本伪装机制。
