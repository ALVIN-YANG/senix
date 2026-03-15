# Contributing to Senix Gateway

感谢您对 Senix Gateway 项目的兴趣！我们欢迎并感谢任何形式的贡献，包括但不限于提交 bug 报告、功能请求、文档改进、代码提交等。

## 📋 行为准则

请阅读并遵守我们的 [Code of Conduct](CODE_OF_CONDUCT.md)，保持友善和尊重。

## 🚀 如何贡献

### 报告 Bug

如果您发现了 bug，请使用 GitHub Issues 报告。请包含以下信息：

- 清晰的 bug 描述
- 复现步骤
- 期望行为 vs 实际行为
- 环境信息（操作系统、Nginx 版本、Go 版本等）
- 相关日志或截图

### 功能请求

我们非常欢迎新功能的建议！请使用 Git Issues 并提供：

- 详细的功能描述
- 使用场景
- 可能的实现方案（可选）

### 代码贡献

#### 开发环境设置

```bash
# 克隆仓库
git clone https://github.com/ALVIN-YANG/senix.git
cd senix

# 安装 Go 依赖
go mod download

# 安装前端依赖
cd web
npm install
```

#### 代码规范

- **Go**: 遵循 standard Go formatting，使用 `gofmt` 和 `golint`
- **React**: 遵循 ESLint 规则，使用函数式组件和 Hooks
- **提交信息**: 使用清晰的提交信息，参考 [Conventional Commits](https://www.conventionalcommits.org/)

#### Pull Request 流程

1. Fork 仓库并创建分支
   ```bash
   git checkout -b feature/your-feature-name
   # 或
   git checkout -b fix/bug-description
   ```

2. 进行开发并提交更改

3. 确保代码通过测试
   ```bash
   # 后端测试
   go test ./...
   
   # 前端测试
   cd web && npm run test
   ```

4. Push 到您的 Fork 并创建 Pull Request

5. 填写 PR 模板，提供清晰的描述

## 🏗️ 项目结构

```
senix/
├── cmd/              # 应用入口
├── configs/          # 配置文件
├── internal/         # 内部包
├── pkg/              # 公共包
├── web/              # React 前端
└── install.sh        # 安装脚本
```

## 🧪 测试

我们重视代码测试。请确保：

- 新功能包含相应的单元测试
- Bug 修复包含回归测试
- 前端组件包含组件测试

## 📝 文档

文档改进同样重要！如果您：

- 发现文档错误
- 想要添加新的文档页面
- 有更好的文档建议

请随时提交 PR 或 Issue。

## 💬 交流

- GitHub Issues: 问题反馈和功能讨论
- GitHub Discussions: 一般性讨论和问答

## 📜 许可证

通过贡献代码，您同意您的贡献将遵循 [Apache License 2.0](LICENSE)。

---

感谢您的贡献！🎉
