# Senix 技术架构方案

## 1. 项目概述

Senix 是一个灵活的网关管理解决方案，支持三种工作模式，兼容用户现有的 Nginx 环境。

### 1.1 三种工作模式

```
┌─────────────────────────────────────────────────────────────────┐
│                    三种工作模式                                 │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  模式 1: 独立代理模式 (Standalone)                               │
│  ├─ Senix 作为完整网关运行                                       │
│  ├─ 内置 HTTP/HTTPS 服务器                                       │
│  ├─ SSL 终结、WAF、限流全部内置                                  │
│  └─ 适合：无 Nginx 或愿意替换 Nginx 的用户                        │
│                                                                 │
│  模式 2: 证书管理模式 (CertOnly)                                 │
│  ├─ 仅管理 SSL 证书                                              │
│  ├─ 自动申请 Let's Encrypt 证书                                  │
│  ├─ 证书导出到用户 Nginx 目录                                    │
│  └─ 适合：已有 Nginx，只想自动管理证书                            │
│                                                                 │
│  模式 3: 配置生成模式 (ConfigOnly)                               │
│  ├─ 生成完整 Nginx 配置文件                                      │
│  ├─ 包含反向代理、SSL、WAF、限流配置                             │
│  ├─ 导出到用户 Nginx 配置目录                                    │
│  └─ 适合：已有 Nginx，想统一管理配置                              │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 1.2 核心设计理念

- **统一部署**：一个 Senix 实例支持所有模式
- **动态切换**：通过 Web 界面切换站点工作模式
- **零侵入**：不修改用户现有 Nginx 配置
- **实时同步**：IP黑名单等规则无需 reload 即可生效

---

## 2. 系统架构

### 2.1 整体架构图

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           Senix 统一实例                                 │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                        Web 管理界面 (Vue 3)                      │   │
│  │  ├─ 仪表盘                                                       │   │
│  │  ├─ 站点管理（选择工作模式）                                      │   │
│  │  ├─ 证书管理                                                     │   │
│  │  ├─ 安全策略（WAF/限流/IP黑名单）                                 │   │
│  │  └─ 系统设置                                                     │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│                              │                                          │
│                              ▼                                          │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                     API 网关层 (Gin)                             │   │
│  │  ├─ /api/sites      站点管理 API                                 │   │
│  │  ├─ /api/certs      证书管理 API                                 │   │
│  │  ├─ /api/security   安全策略 API                                 │   │
│  │  ├─ /api/config     配置导出 API                                 │   │
│  │  └─ /api/system     系统管理 API                                 │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│                              │                                          │
│                              ▼                                          │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                     核心服务层 (Services)                        │   │
│  │                                                                  │   │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐             │   │
│  │  │ SiteService │  │CertService  │  │SecuritySvc  │             │   │
│  │  │ 站点管理    │  │证书管理     │  │安全管理     │             │   │
│  │  │             │  │             │  │             │             │   │
│  │  │ - 工作模式  │  │ - 申请证书  │  │ - IP黑名单  │             │   │
│  │  │ - 代理配置  │  │ - 续期管理  │  │ - 限流器    │             │   │
│  │  │ - 健康检查  │  │ - 导出证书  │  │ - WAF规则   │             │   │
│  │  └─────────────┘  └─────────────┘  └─────────────┘             │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│                              │                                          │
│                              ▼                                          │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                     工作模式执行层 (Executors)                    │   │
│  │                                                                  │   │
│  │  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐ │   │
│  │  │  独立代理模式    │  │  证书管理模式    │  │  配置生成模式    │ │   │
│  │  │  (Standalone)   │  │  (CertOnly)     │  │  (ConfigOnly)   │ │   │
│  │  │                 │  │                 │  │                 │ │   │
│  │  │ 内置 HTTP 服务器 │  │ 仅管理证书      │  │ 生成 Nginx 配置 │ │   │
│  │  │ 直接处理请求    │  │ 导出到指定目录  │  │ 导出到指定目录  │ │   │
│  │  │ SSL 终结        │  │ 可选自动重载    │  │ 可选自动重载    │ │   │
│  │  │ WAF/限流处理    │  │                │  │                │ │   │
│  │  │ 反向代理        │  │                │  │                │ │   │
│  │  └─────────────────┘  └─────────────────┘  └─────────────────┘ │   │
│  │                                                                  │   │
│  │  每个站点可独立选择工作模式！                                      │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│                              │                                          │
│                              ▼                                          │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                     数据持久层                                   │   │
│  │  ├─ SQLite (配置、证书元数据)                                    │   │
│  │  ├─ 文件系统 (证书文件、配置文件、日志)                           │   │
│  │  └─ 内存 (运行时缓存、限流计数器)                                 │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### 2.2 技术栈

| 层级 | 技术选型 | 说明 |
|------|---------|------|
| **前端** | Vue 3 + Element Plus + TypeScript | 现代化管理界面 |
| **API 层** | Gin | 高性能 Go Web 框架 |
| **服务层** | 纯 Go 实现 | 业务逻辑 |
| **数据库** | SQLite (默认) / PostgreSQL (可选) | 轻量级存储 |
| **证书** | lego | Let's Encrypt 客户端 |
| **WAF (独立模式)** | Coraza | Go 语言 WAF 引擎 |
| **WAF (配置模式)** | ModSecurity | Nginx WAF 模块 |
| **部署** | Docker / Docker Compose | 容器化部署 |

---

## 3. 数据模型设计

### 3.1 站点模型 (Site)

```go
type WorkMode string

const (
    WorkModeStandalone WorkMode = "standalone"  // 独立代理模式
    WorkModeCertOnly   WorkMode = "cert_only"    // 证书管理模式
    WorkModeConfigOnly WorkMode = "config_only"  // 配置生成模式
)

type Site struct {
    ID          uint      `json:"id" gorm:"primaryKey"`
    Name        string    `json:"name" gorm:"size:100;not null"`
    Domain      string    `json:"domain" gorm:"size:255;uniqueIndex;not null"`
    
    // 工作模式（核心字段）
    WorkMode    WorkMode  `json:"work_mode" gorm:"size:20;default:'standalone'"`
    
    // 基础配置
    Port        int       `json:"port" gorm:"default:80"`
    SSLEnabled  bool      `json:"ssl_enabled" gorm:"default:false"`
    SSLAuto     bool      `json:"ssl_auto" gorm:"default:false"`
    
    // 后端配置
    Upstream    string    `json:"upstream" gorm:"size:500"`
    LoadBalance string    `json:"load_balance" gorm:"size:50;default:round_robin"`
    
    // 证书关联
    CertID      *uint     `json:"cert_id" gorm:"index"`
    Cert        *Certificate `json:"cert,omitempty" gorm:"foreignKey:CertID"`
    
    // 安全功能开关
    EnableWAF       bool `json:"enable_waf" gorm:"default:false"`
    EnableRateLimit bool `json:"enable_rate_limit" gorm:"default:false"`
    EnableIPBlacklist bool `json:"enable_ip_blacklist" gorm:"default:false"`
    
    // 安全功能配置
    WAFMode         string `json:"waf_mode" gorm:"size:20;default:detection"`
    RateLimitReqPS  int    `json:"rate_limit_req_ps" gorm:"default:100"`
    RateLimitBurst  int    `json:"rate_limit_burst" gorm:"default:150"`
    
    // 证书管理模式配置
    CertExportPath  string `json:"cert_export_path" gorm:"size:500"`
    AutoReloadNginx bool   `json:"auto_reload_nginx" gorm:"default:false"`
    NginxReloadCmd  string `json:"nginx_reload_cmd" gorm:"size:500"`
    
    // 配置生成模式配置
    ConfigExportPath string `json:"config_export_path" gorm:"size:500"`
    NginxTemplate    string `json:"nginx_template" gorm:"type:text"`
    
    // 状态
    Status      string    `json:"status" gorm:"size:20;default:inactive"`
    ErrorMsg    string    `json:"error_msg" gorm:"size:500"`
    
    Description string    `json:"description" gorm:"size:500"`
    CreatedAt   time.Time `json:"created_at"`
    UpdatedAt   time.Time `json:"updated_at"`
}
```

### 3.2 证书模型 (Certificate)

```go
type CertSource string

const (
    CertSourceACME   CertSource = "acme"
    CertSourceUpload CertSource = "upload"
    CertSourceImport CertSource = "import"
)

type Certificate struct {
    ID          uint       `json:"id" gorm:"primaryKey"`
    Domain      string     `json:"domain" gorm:"size:255;index;not null"`
    Source      CertSource `json:"source" gorm:"size:20;default:'acme'"`
    
    // ACME 配置
    ACMEEmail   string     `json:"acme_email" gorm:"size:255"`
    ACMECADir   string     `json:"acme_ca_dir" gorm:"size:255"`
    DNSProvider string     `json:"dns_provider" gorm:"size:100"`
    DNSConfig   string     `json:"dns_config" gorm:"type:text"`
    
    // 证书文件路径
    CertPath    string     `json:"cert_path" gorm:"size:500"`
    KeyPath     string     `json:"key_path" gorm:"size:500"`
    ChainPath   string     `json:"chain_path" gorm:"size:500"`
    
    // 证书信息
    Issuer      string     `json:"issuer" gorm:"size:255"`
    Subject     string     `json:"subject" gorm:"size:255"`
    SANs        string     `json:"sans" gorm:"size:500"`
    NotBefore   time.Time  `json:"not_before"`
    NotAfter    time.Time  `json:"not_after"`
    SerialNumber string    `json:"serial_number" gorm:"size:100"`
    
    // 自动续期
    AutoRenew   bool       `json:"auto_renew" gorm:"default:true"`
    RenewedAt   *time.Time `json:"renewed_at"`
    
    // 状态
    Status      string     `json:"status" gorm:"size:20;default:pending"`
    ErrorMsg    string     `json:"error_msg" gorm:"size:500"`
    
    CreatedAt   time.Time  `json:"created_at"`
    UpdatedAt   time.Time  `json:"updated_at"`
}
```

---

## 4. 核心模块设计

### 4.1 证书管理模块 (CertManager)

```go
type CertManager struct {
    dataDir       string
    renewInterval time.Duration
    certificates  map[string]*Certificate
    mu            sync.RWMutex
    stopCh        chan struct{}
}

// 核心方法
func (cm *CertManager) Issue(domain, email string, dnsProvider string) (*Certificate, error)
func (cm *CertManager) Renew(certID uint) (*Certificate, error)
func (cm *CertManager) Export(domain string) (certPEM, keyPEM []byte, error)
func (cm *CertManager) GetCertificate(domain string) (*Certificate, error)
```

### 4.2 配置生成模块 (ConfigGenerator)

```go
type ConfigGenerator struct {
    templateDir string
}

// 生成 Nginx 配置
func (cg *ConfigGenerator) GenerateSiteConfig(site *Site) (string, error)
func (cg *ConfigGenerator) GenerateSSLConfig(site *Site) (string, error)
func (cg *ConfigGenerator) GenerateWAFConfig(site *Site) (string, error)
func (cg *ConfigGenerator) GenerateRateLimitConfig(site *Site) (string, error)
```

### 4.3 实时同步模块 (RealtimeSyncer)

```go
type RealtimeSyncer struct {
    ipBlacklist *IPBlacklistManager
    rateLimit   *RateLimitController
    wafRules    *WAFRulesManager
}

// 实时同步方法
func (rs *RealtimeSyncer) SyncIPBlacklist(ips []string) error
func (rs *RealtimeSyncer) UpdateRateLimit(zone string, rps, burst int) error
func (rs *RealtimeSyncer) UpdateWAFRules(rules []WAFRule) error
```

---

## 5. API 设计

### 5.1 站点管理 API

```
GET    /api/sites              # 获取站点列表
POST   /api/sites              # 创建站点
GET    /api/sites/:id          # 获取站点详情
PUT    /api/sites/:id          # 更新站点
DELETE /api/sites/:id          # 删除站点
POST   /api/sites/:id/enable   # 启用站点
POST   /api/sites/:id/disable  # 禁用站点
GET    /api/sites/:id/config   # 预览配置
```

### 5.2 证书管理 API

```
GET    /api/certs              # 获取证书列表
POST   /api/certs              # 申请证书
GET    /api/certs/:id          # 获取证书详情
DELETE /api/certs/:id          # 删除证书
POST   /api/certs/:id/renew    # 手动续期
GET    /api/certs/:id/download # 下载证书
```

### 5.3 安全策略 API

```
GET    /api/security/ip-blacklist     # 获取 IP 黑名单
POST   /api/security/ip-blacklist     # 添加 IP 到黑名单
DELETE /api/security/ip-blacklist/:ip # 从黑名单移除

GET    /api/security/rate-limits      # 获取限流策略
PUT    /api/security/rate-limits/:id  # 更新限流策略

GET    /api/security/waf-rules        # 获取 WAF 规则
POST   /api/security/waf-rules        # 创建 WAF 规则
PUT    /api/security/waf-rules/:id    # 更新 WAF 规则
DELETE /api/security/waf-rules/:id    # 删除 WAF 规则
```

---

## 6. 部署方案

### 6.1 Docker Compose 部署

```yaml
version: '3.8'

services:
  senix:
    image: senix:latest
    container_name: senix
    restart: unless-stopped
    ports:
      - "80:80"      # HTTP
      - "443:443"    # HTTPS
      - "8080:8080"  # 管理界面
    volumes:
      - ./data:/app/data
      - ./logs:/app/logs
      # 可选：导出到宿主机的 Nginx 目录
      - /path/to/nginx/certs:/app/data/certs-export
      - /path/to/nginx/conf.d:/app/data/config-export
    environment:
      - SENIX_MODE=full
      - SENIX_DATA_DIR=/app/data
      - SENIX_LOG_LEVEL=info
```

### 6.2 目录结构

```
data/
├── certs/              # 证书存储
│   ├── example.com.crt
│   └── example.com.key
├── configs/            # 生成的配置
│   └── sites/
├── db/                 # SQLite 数据库
│   └── senix.db
└── logs/               # 日志文件
    ├── senix.log
    └── access.log
```

---

## 7. 开发路线图

### Phase 1: 核心基础 (Week 1-4)
- M1: 项目基础架构搭建
- M2: 证书管理核心模块
- M3: 站点管理与配置生成

### Phase 2: 管理界面 (Week 5-7)
- M4: Web管理界面

### Phase 3: 高级功能 (Week 8-12)
- M5: 安全功能模块
- M6: 独立代理模式
- M7: 测试与优化

---

## 8. 风险评估

| 风险 | 影响 | 缓解措施 |
|------|------|---------|
| Coraza 性能不足 | 高 | 提供 ModSecurity 作为备选 |
| 证书申请失败 | 中 | 完善的错误处理和重试机制 |
| Nginx 配置错误 | 高 | 配置验证（nginx -t）和自动回滚 |
| 用户权限问题 | 中 | 详细的权限检查和提示 |

---

## 9. 成功指标

- [ ] 支持三种工作模式无缝切换
- [ ] 证书自动续期成功率 > 99%
- [ ] 配置生成零错误
- [ ] 独立代理模式性能 > 5000 QPS
- [ ] 代码测试覆盖率 > 70%

---

**文档版本**: v1.0  
**最后更新**: 2024-01-15  
**作者**: Senix Team
