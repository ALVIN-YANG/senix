# Senix - 高性能 Nginx 网关

基于 Nginx 数据面 + Go 控制面的高性能网关解决方案。

## 架构设计

```
┌─────────────────────────────────────────────────────────┐
│                      用户请求                            │
└─────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────┐
│  Nginx (数据面) - 高性能处理所有流量                      │
│  ├── SSL 证书 (由 Senix 管理)                           │
│  ├── WAF 规则 (由 Senix 生成配置)                        │
│  ├── 反向代理                                           │
│  └── 静态文件服务                                        │
└─────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────┐
│  后端服务                                                │
└─────────────────────────────────────────────────────────┘

        ↑ 配置同步 (HTTP API / 文件)
        │
┌─────────────────────────────────────────────────────────┐
│  Senix 控制面 (Go) - 管理功能                            │
│  ├── Web 管理界面 (端口 8080)                            │
│  ├── 证书自动申请/续期 (Let's Encrypt)                   │
│  ├── WAF 规则管理                                        │
│  └── 监控统计                                            │
└─────────────────────────────────────────────────────────┘
```

## 核心特性

- **高性能**: Nginx 原生处理流量，零性能损耗
- **自动证书**: Let's Encrypt 自动申请和续期
- **WAF 防护**: 基于 Coraza 的 Web 应用防火墙
- **可视化管理**: 现代化的 Web 管理界面
- **多站点支持**: 轻松管理多个域名和站点
- **热重载**: 配置变更无需重启

## 技术栈

- **数据面**: Nginx + ModSecurity (可选)
- **控制面**: Go + Gin + GORM
- **前端**: Vue 3 + Element Plus
- **数据库**: SQLite (默认) / PostgreSQL (可选)
- **证书**: lego (Let's Encrypt 客户端)

## 快速开始

### 使用 Docker Compose 部署

```bash
cd senix
docker-compose up -d
```

访问管理界面: http://localhost:8080

### 手动部署

1. 启动 Nginx
```bash
docker-compose -f deployments/docker/nginx.yml up -d
```

2. 启动 Senix 控制面
```bash
cd cmd/senix
go run main.go
```

## 项目结构

```
senix/
├── cmd/senix/           # 主程序入口
├── internal/
│   ├── api/            # HTTP API 接口
│   ├── config/         # 配置管理
│   ├── models/         # 数据模型
│   ├── services/       # 业务逻辑
│   ├── waf/            # WAF 引擎 (基于 Coraza)
│   ├── cert/           # 证书管理 (基于 lego)
│   └── nginx/          # Nginx 配置管理
├── web/                # 前端代码 (Vue 3)
├── deployments/        # 部署配置
├── configs/            # 配置文件模板
└── scripts/            # 辅助脚本
```

## 开源协议

Apache License 2.0

## 致谢

本项目参考并使用了以下开源项目：
- [Coraza](https://github.com/corazawaf/coraza) - WAF 引擎
- [lego](https://github.com/go-acme/lego) - Let's Encrypt 客户端
- [SamWaf](https://github.com/samwafgo/SamWaf) - 架构参考
