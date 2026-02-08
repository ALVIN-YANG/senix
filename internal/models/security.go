package models

import "time"

// WAFRule WAF 规则模型
type WAFRule struct {
	ID          uint   `json:"id" gorm:"primaryKey"`
	SiteID      *uint  `json:"site_id" gorm:"index"` // nil 表示全局规则
	Name        string `json:"name" gorm:"size:100;not null"`
	Description string `json:"description" gorm:"size:500"`
	
	// 规则配置
	RuleType string `json:"rule_type" gorm:"size:50;not null"` // sql_injection, xss, lfi, rfi, rce, custom
	Phase    int    `json:"phase" gorm:"default:2"`             // 1: request headers, 2: request body
	Pattern  string `json:"pattern" gorm:"type:text;not null"`  // 正则表达式或规则内容
	Action   string `json:"action" gorm:"size:50;default:block"` // block, log, allow
	
	// 状态
	Enabled  bool   `json:"enabled" gorm:"default:true"`
	Priority int    `json:"priority" gorm:"default:100"`
	
	CreatedAt time.Time `json:"created_at"`
	UpdatedAt time.Time `json:"updated_at"`
}

// TableName 指定表名
func (WAFRule) TableName() string {
	return "waf_rules"
}

// IPBlacklist IP 黑名单模型
type IPBlacklist struct {
	ID      uint   `json:"id" gorm:"primaryKey"`
	SiteID  *uint  `json:"site_id" gorm:"index"` // nil 表示全局黑名单
	IP      string `json:"ip" gorm:"size:50;not null"`
	Reason  string `json:"reason" gorm:"size:255"`
	Expires *time.Time `json:"expires"` // nil 表示永久
	
	CreatedAt time.Time `json:"created_at"`
	UpdatedAt time.Time `json:"updated_at"`
}

// TableName 指定表名
func (IPBlacklist) TableName() string {
	return "ip_blacklists"
}

// IsExpired 是否已过期
func (i *IPBlacklist) IsExpired() bool {
	if i.Expires == nil {
		return false
	}
	return time.Now().After(*i.Expires)
}

// RateLimit 限流配置模型
type RateLimit struct {
	ID      uint   `json:"id" gorm:"primaryKey"`
	SiteID  uint   `json:"site_id" gorm:"index"`
	Name    string `json:"name" gorm:"size:100;not null"`
	
	// 限流配置
	RequestsPerSecond int `json:"requests_per_second" gorm:"default:100"`
	Burst             int `json:"burst" gorm:"default:150"`
	
	// 作用范围
	Path    string `json:"path" gorm:"size:255;default:'/'"`
	Methods string `json:"methods" gorm:"size:100;default:'*'"` // * 或 GET,POST,PUT
	
	// 行为
	Action   string `json:"action" gorm:"size:50;default:delay"` // delay, reject
	RejectCode int `json:"reject_code" gorm:"default:429"`
	
	Enabled   bool      `json:"enabled" gorm:"default:true"`
	CreatedAt time.Time `json:"created_at"`
	UpdatedAt time.Time `json:"updated_at"`
}

// TableName 指定表名
func (RateLimit) TableName() string {
	return "rate_limits"
}
