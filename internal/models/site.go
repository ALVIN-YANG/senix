package models

import (
	"time"
)

// WorkMode 工作模式
type WorkMode string

const (
	WorkModeStandalone WorkMode = "standalone" // 独立代理模式
	WorkModeCertOnly   WorkMode = "cert_only"   // 证书管理模式
	WorkModeConfigOnly WorkMode = "config_only" // 配置生成模式
)

// Site 站点模型
type Site struct {
	ID     uint   `json:"id" gorm:"primaryKey"`
	Name   string `json:"name" gorm:"size:100;not null"`
	Domain string `json:"domain" gorm:"size:255;uniqueIndex;not null"`

	// 工作模式
	WorkMode WorkMode `json:"work_mode" gorm:"size:20;default:'standalone'"`

	// 基础配置
	Port       int  `json:"port" gorm:"default:80"`
	SSLEnabled bool `json:"ssl_enabled" gorm:"default:false"`
	SSLAuto    bool `json:"ssl_auto" gorm:"default:false"`

	// 后端配置
	Upstream    string `json:"upstream" gorm:"size:500"`
	LoadBalance string `json:"load_balance" gorm:"size:50;default:round_robin"`

	// 证书关联
	CertID *uint        `json:"cert_id" gorm:"index"`
	Cert   *Certificate `json:"cert,omitempty" gorm:"foreignKey:CertID"`

	// 安全功能开关
	EnableWAF         bool `json:"enable_waf" gorm:"default:false"`
	EnableRateLimit   bool `json:"enable_rate_limit" gorm:"default:false"`
	EnableIPBlacklist bool `json:"enable_ip_blacklist" gorm:"default:false"`

	// 安全功能配置
	WAFMode        string `json:"waf_mode" gorm:"size:20;default:detection"` // detection, blocking
	RateLimitReqPS int    `json:"rate_limit_req_ps" gorm:"default:100"`
	RateLimitBurst int    `json:"rate_limit_burst" gorm:"default:150"`

	// 证书管理模式配置
	CertExportPath  string `json:"cert_export_path" gorm:"size:500"`
	AutoReloadNginx bool   `json:"auto_reload_nginx" gorm:"default:false"`
	NginxReloadCmd  string `json:"nginx_reload_cmd" gorm:"size:500"`

	// 配置生成模式配置
	ConfigExportPath string `json:"config_export_path" gorm:"size:500"`
	NginxTemplate    string `json:"nginx_template" gorm:"type:text"`

	// 状态
	Status   string `json:"status" gorm:"size:20;default:inactive"` // inactive, active, error
	ErrorMsg string `json:"error_msg" gorm:"size:500"`

	Description string    `json:"description" gorm:"size:500"`
	CreatedAt   time.Time `json:"created_at"`
	UpdatedAt   time.Time `json:"updated_at"`
}

// TableName 指定表名
func (Site) TableName() string {
	return "sites"
}

// IsActive 是否激活
func (s *Site) IsActive() bool {
	return s.Status == "active"
}

// SiteRequest 创建/更新站点请求
type SiteRequest struct {
	Name        string   `json:"name" binding:"required,max=100"`
	Domain      string   `json:"domain" binding:"required,fqdn"`
	WorkMode    WorkMode `json:"work_mode" binding:"oneof=standalone cert_only config_only"`
	Port        int      `json:"port" binding:"min=1,max=65535"`
	SSLEnabled  bool     `json:"ssl_enabled"`
	SSLAuto     bool     `json:"ssl_auto"`
	Upstream    string   `json:"upstream"`
	LoadBalance string   `json:"load_balance"`
	CertID      *uint    `json:"cert_id"`

	// 安全功能
	EnableWAF         bool   `json:"enable_waf"`
	EnableRateLimit   bool   `json:"enable_rate_limit"`
	EnableIPBlacklist bool   `json:"enable_ip_blacklist"`
	WAFMode           string `json:"waf_mode" binding:"oneof=detection blocking"`
	RateLimitReqPS    int    `json:"rate_limit_req_ps"`
	RateLimitBurst    int    `json:"rate_limit_burst"`

	// 证书管理模式
	CertExportPath  string `json:"cert_export_path"`
	AutoReloadNginx bool   `json:"auto_reload_nginx"`
	NginxReloadCmd  string `json:"nginx_reload_cmd"`

	// 配置生成模式
	ConfigExportPath string `json:"config_export_path"`
	NginxTemplate    string `json:"nginx_template"`

	Description string `json:"description"`
}

// SiteResponse 站点响应
type SiteResponse struct {
	ID                uint      `json:"id"`
	Name              string    `json:"name"`
	Domain            string    `json:"domain"`
	WorkMode          WorkMode  `json:"work_mode"`
	Port              int       `json:"port"`
	SSLEnabled        bool      `json:"ssl_enabled"`
	SSLAuto           bool      `json:"ssl_auto"`
	Upstream          string    `json:"upstream"`
	LoadBalance       string    `json:"load_balance"`
	CertID            *uint     `json:"cert_id"`
	Cert              *CertificateResponse `json:"cert,omitempty"`
	EnableWAF         bool      `json:"enable_waf"`
	EnableRateLimit   bool      `json:"enable_rate_limit"`
	EnableIPBlacklist bool      `json:"enable_ip_blacklist"`
	WAFMode           string    `json:"waf_mode"`
	RateLimitReqPS    int       `json:"rate_limit_req_ps"`
	RateLimitBurst    int       `json:"rate_limit_burst"`
	CertExportPath    string    `json:"cert_export_path,omitempty"`
	AutoReloadNginx   bool      `json:"auto_reload_nginx,omitempty"`
	ConfigExportPath  string    `json:"config_export_path,omitempty"`
	Status            string    `json:"status"`
	ErrorMsg          string    `json:"error_msg,omitempty"`
	Description       string    `json:"description"`
	CreatedAt         time.Time `json:"created_at"`
	UpdatedAt         time.Time `json:"updated_at"`
}

// ToResponse 转换为响应格式
func (s *Site) ToResponse() *SiteResponse {
	resp := &SiteResponse{
		ID:                s.ID,
		Name:              s.Name,
		Domain:            s.Domain,
		WorkMode:          s.WorkMode,
		Port:              s.Port,
		SSLEnabled:        s.SSLEnabled,
		SSLAuto:           s.SSLAuto,
		Upstream:          s.Upstream,
		LoadBalance:       s.LoadBalance,
		CertID:            s.CertID,
		EnableWAF:         s.EnableWAF,
		EnableRateLimit:   s.EnableRateLimit,
		EnableIPBlacklist: s.EnableIPBlacklist,
		WAFMode:           s.WAFMode,
		RateLimitReqPS:    s.RateLimitReqPS,
		RateLimitBurst:    s.RateLimitBurst,
		CertExportPath:    s.CertExportPath,
		AutoReloadNginx:   s.AutoReloadNginx,
		ConfigExportPath:  s.ConfigExportPath,
		Status:            s.Status,
		ErrorMsg:          s.ErrorMsg,
		Description:       s.Description,
		CreatedAt:         s.CreatedAt,
		UpdatedAt:         s.UpdatedAt,
	}

	// 如果有关联证书，转换证书响应
	if s.Cert != nil {
		resp.Cert = s.Cert.ToResponse()
	}

	return resp
}

// NginxConfigData Nginx 配置数据
type NginxConfigData struct {
	ServerName        string
	Port              int
	SSLPort           int
	SSLEnabled        bool
	CertPath          string
	KeyPath           string
	Upstream          string
	LoadBalance       string
	EnableWAF         bool
	EnableRateLimit   bool
	EnableIPBlacklist bool
	WAFMode           string
	RateLimitReqPS    int
	RateLimitBurst    int
	IPBlacklist       []string
	Locations         []LocationConfig
}

// LocationConfig Location 配置
type LocationConfig struct {
	Path     string
	ProxyPass string
	Options  map[string]string
}

// SiteStats 站点统计
type SiteStats struct {
	Total      int64 `json:"total"`
	Active     int64 `json:"active"`
	Inactive   int64 `json:"inactive"`
	WithSSL    int64 `json:"with_ssl"`
	Standalone int64 `json:"standalone"`
	CertOnly   int64 `json:"cert_only"`
	ConfigOnly int64 `json:"config_only"`
}
