package models

import (
	"time"
)

// CertSource 证书来源
type CertSource string

const (
	CertSourceACME   CertSource = "acme"    // ACME 自动申请
	CertSourceUpload CertSource = "upload"  // 手动上传
	CertSourceImport CertSource = "import"  // 外部导入
)

// Certificate 证书模型
type Certificate struct {
	ID       uint       `json:"id" gorm:"primaryKey"`
	Domain   string     `json:"domain" gorm:"size:255;index;not null"`
	Source   CertSource `json:"source" gorm:"size:20;default:'acme'"`

	// ACME 配置
	ACMEEmail   string `json:"acme_email" gorm:"size:255"`
	ACMECADir   string `json:"acme_ca_dir" gorm:"size:255"`
	DNSProvider string `json:"dns_provider" gorm:"size:100"`
	DNSConfig   string `json:"dns_config" gorm:"type:text"` // JSON 格式

	// 证书文件路径
	CertPath  string `json:"cert_path" gorm:"size:500"`
	KeyPath   string `json:"key_path" gorm:"size:500"`
	ChainPath string `json:"chain_path" gorm:"size:500"`

	// 证书信息
	Issuer       string    `json:"issuer" gorm:"size:255"`
	Subject      string    `json:"subject" gorm:"size:255"`
	SANs         string    `json:"sans" gorm:"size:500"` // 逗号分隔的域名列表
	NotBefore    time.Time `json:"not_before"`
	NotAfter     time.Time `json:"not_after"`
	SerialNumber string    `json:"serial_number" gorm:"size:100"`

	// 自动续期
	AutoRenew bool       `json:"auto_renew" gorm:"default:true"`
	RenewedAt *time.Time `json:"renewed_at"`

	// 状态
	Status   string `json:"status" gorm:"size:20;default:pending"` // pending, active, expired, error
	ErrorMsg string `json:"error_msg" gorm:"size:500"`

	CreatedAt time.Time `json:"created_at"`
	UpdatedAt time.Time `json:"updated_at"`
}

// TableName 指定表名
func (Certificate) TableName() string {
	return "certificates"
}

// IsExpired 是否已过期
func (c *Certificate) IsExpired() bool {
	return time.Now().After(c.NotAfter)
}

// DaysUntilExpiry 距离过期还有多少天
func (c *Certificate) DaysUntilExpiry() int {
	duration := c.NotAfter.Sub(time.Now())
	return int(duration.Hours() / 24)
}

// CertificateInfo 证书信息（用于解析后返回）
type CertificateInfo struct {
	Issuer       string    `json:"issuer"`
	Subject      string    `json:"subject"`
	SANs         []string  `json:"sans"`
	NotBefore    time.Time `json:"not_before"`
	NotAfter     time.Time `json:"not_after"`
	SerialNumber string    `json:"serial_number"`
}

// DNSProviderConfig DNS 提供商配置
type DNSProviderConfig struct {
	Provider   string            `json:"provider"`    // cloudflare, alidns, tencentcloud
	Config     map[string]string `json:"config"`      // 提供商特定的配置
}

// ACMERequest ACME 证书申请请求
type ACMERequest struct {
	Domain      string            `json:"domain" binding:"required,fqdn"`
	Email       string            `json:"email" binding:"required,email"`
	Challenge   string            `json:"challenge" binding:"oneof=http-01 dns-01"`
	DNSProvider string            `json:"dns_provider,omitempty"`
	DNSConfig   map[string]string `json:"dns_config,omitempty"`
	AutoRenew   bool              `json:"auto_renew"`
}

// CertificateResponse 证书响应
type CertificateResponse struct {
	ID           uint       `json:"id"`
	Domain       string     `json:"domain"`
	Source       string     `json:"source"`
	Issuer       string     `json:"issuer"`
	Subject      string     `json:"subject"`
	SANs         []string   `json:"sans"`
	NotBefore    time.Time  `json:"not_before"`
	NotAfter     time.Time  `json:"not_after"`
	SerialNumber string     `json:"serial_number"`
	AutoRenew    bool       `json:"auto_renew"`
	Status       string     `json:"status"`
	DaysLeft     int        `json:"days_left"`
	CreatedAt    time.Time  `json:"created_at"`
	UpdatedAt    time.Time  `json:"updated_at"`
}

// ToResponse 转换为响应格式
func (c *Certificate) ToResponse() *CertificateResponse {
	resp := &CertificateResponse{
		ID:           c.ID,
		Domain:       c.Domain,
		Source:       string(c.Source),
		Issuer:       c.Issuer,
		Subject:      c.Subject,
		NotBefore:    c.NotBefore,
		NotAfter:     c.NotAfter,
		SerialNumber: c.SerialNumber,
		AutoRenew:    c.AutoRenew,
		Status:       c.Status,
		DaysLeft:     c.DaysUntilExpiry(),
		CreatedAt:    c.CreatedAt,
		UpdatedAt:    c.UpdatedAt,
	}

	// 解析 SANs
	if c.SANs != "" {
		// 简单按逗号分割
		resp.SANs = splitAndTrim(c.SANs, ",")
	}

	return resp
}

// splitAndTrim 分割字符串并去除空白
func splitAndTrim(s, sep string) []string {
	parts := []string{}
	for _, p := range splitString(s, sep) {
		p = trimSpace(p)
		if p != "" {
			parts = append(parts, p)
		}
	}
	return parts
}

// splitString 分割字符串
func splitString(s, sep string) []string {
	result := []string{}
	start := 0
	for i := 0; i < len(s); i++ {
		if i+len(sep) <= len(s) && s[i:i+len(sep)] == sep {
			result = append(result, s[start:i])
			start = i + len(sep)
		}
	}
	result = append(result, s[start:])
	return result
}

// trimSpace 去除首尾空白
func trimSpace(s string) string {
	start := 0
	end := len(s)
	for start < end && (s[start] == ' ' || s[start] == '\t' || s[start] == '\n' || s[start] == '\r') {
		start++
	}
	for end > start && (s[end-1] == ' ' || s[end-1] == '\t' || s[end-1] == '\n' || s[end-1] == '\r') {
		end--
	}
	return s[start:end]
}
