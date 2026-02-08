package nginx

import (
	"bytes"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"text/template"

	"senix/internal/config"
	"senix/internal/logger"
	"senix/internal/models"
)

// Generator Nginx 配置生成器
type Generator struct {
	configDir   string
	templateDir string
	config      *config.NginxConfig
}

// NewGenerator 创建配置生成器
func NewGenerator(cfg *config.NginxConfig) *Generator {
	return &Generator{
		configDir:   cfg.ConfigDir,
		templateDir: cfg.TemplateDir,
		config:      cfg,
	}
}

// Initialize 初始化
func (g *Generator) Initialize() error {
	// 确保配置目录存在
	if err := os.MkdirAll(g.configDir, 0755); err != nil {
		return fmt.Errorf("create config dir failed: %w", err)
	}

	// 确保模板目录存在
	if err := os.MkdirAll(g.templateDir, 0755); err != nil {
		return fmt.Errorf("create template dir failed: %w", err)
	}

	// 创建默认模板
	if err := g.createDefaultTemplates(); err != nil {
		return fmt.Errorf("create default templates failed: %w", err)
	}

	return nil
}

// createDefaultTemplates 创建默认模板
func (g *Generator) createDefaultTemplates() error {
	// 默认站点模板
	defaultTemplate := `{{if .SSLEnabled}}
# HTTP to HTTPS redirect
server {
    listen {{.Port}};
    server_name {{.ServerName}};
    return 301 https://$server_name$request_uri;
}
{{end}}

server {
    {{if .SSLEnabled}}
    listen 443 ssl http2;
    {{else}}
    listen {{.Port}};
    {{end}}
    server_name {{.ServerName}};

    {{if .SSLEnabled}}
    # SSL Configuration
    ssl_certificate {{.CertPath}};
    ssl_certificate_key {{.KeyPath}};
    ssl_protocols TLSv1.2 TLSv1.3;
    ssl_ciphers ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES128-GCM-SHA256:ECDHE-ECDSA-AES256-GCM-SHA384:ECDHE-RSA-AES256-GCM-SHA384;
    ssl_prefer_server_ciphers off;
    ssl_session_cache shared:SSL:10m;
    ssl_session_timeout 1d;
    {{end}}

    # Security Headers
    add_header X-Frame-Options "SAMEORIGIN" always;
    add_header X-Content-Type-Options "nosniff" always;
    add_header X-XSS-Protection "1; mode=block" always;
    add_header Referrer-Policy "strict-origin-when-cross-origin" always;

    {{if .EnableRateLimit}}
    # Rate Limiting
    limit_req_zone $binary_remote_addr zone={{.ServerName}}_limit:10m rate={{.RateLimitReqPS}}r/s;
    limit_req zone={{.ServerName}}_limit burst={{.RateLimitBurst}} nodelay;
    {{end}}

    {{if .EnableIPBlacklist}}
    # IP Blacklist
    {{range .IPBlacklist}}
    deny {{.}};
    {{end}}
    allow all;
    {{end}}

    {{if .EnableWAF}}
    # WAF Configuration
    modsecurity on;
    modsecurity_rules_file /etc/nginx/modsecurity/modsecurity.conf;
    {{end}}

    # Logging
    access_log /var/log/nginx/{{.ServerName}}_access.log;
    error_log /var/log/nginx/{{.ServerName}}_error.log;

    # Locations
    location / {
        {{if .Upstream}}
        proxy_pass {{.Upstream}};
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        
        proxy_connect_timeout 60s;
        proxy_send_timeout 60s;
        proxy_read_timeout 60s;
        
        proxy_buffering on;
        proxy_buffer_size 4k;
        proxy_buffers 8 4k;
        {{else}}
        root /var/www/html;
        index index.html index.htm;
        {{end}}
    }

    # Health check endpoint
    location /health {
        access_log off;
        return 200 "healthy\n";
        add_header Content-Type text/plain;
    }
}
`

	templatePath := filepath.Join(g.templateDir, "default.conf.tmpl")
	if _, err := os.Stat(templatePath); os.IsNotExist(err) {
		if err := os.WriteFile(templatePath, []byte(defaultTemplate), 0644); err != nil {
			return fmt.Errorf("write default template failed: %w", err)
		}
	}

	return nil
}

// GenerateSiteConfig 生成站点配置
func (g *Generator) GenerateSiteConfig(site *models.Site, ipBlacklist []string) (string, error) {
	// 准备模板数据
	data := &models.NginxConfigData{
		ServerName:        site.Domain,
		Port:              site.Port,
		SSLPort:           443,
		SSLEnabled:        site.SSLEnabled,
		Upstream:          site.Upstream,
		LoadBalance:       site.LoadBalance,
		EnableWAF:         site.EnableWAF,
		EnableRateLimit:   site.EnableRateLimit,
		EnableIPBlacklist: site.EnableIPBlacklist,
		WAFMode:           site.WAFMode,
		RateLimitReqPS:    site.RateLimitReqPS,
		RateLimitBurst:    site.RateLimitBurst,
		IPBlacklist:       ipBlacklist,
	}

	// 如果有证书，设置证书路径
	if site.Cert != nil {
		data.CertPath = site.Cert.CertPath
		data.KeyPath = site.Cert.KeyPath
	}

	// 选择模板
	templateName := "default.conf.tmpl"
	if site.NginxTemplate != "" {
		// 使用自定义模板
		templateName = site.NginxTemplate
	}

	templatePath := filepath.Join(g.templateDir, templateName)

	// 读取模板
	tmplContent, err := os.ReadFile(templatePath)
	if err != nil {
		return "", fmt.Errorf("read template failed: %w", err)
	}

	// 解析模板
	tmpl, err := template.New("nginx").Parse(string(tmplContent))
	if err != nil {
		return "", fmt.Errorf("parse template failed: %w", err)
	}

	// 执行模板
	var buf bytes.Buffer
	if err := tmpl.Execute(&buf, data); err != nil {
		return "", fmt.Errorf("execute template failed: %w", err)
	}

	return buf.String(), nil
}

// SaveSiteConfig 保存站点配置到文件
func (g *Generator) SaveSiteConfig(site *models.Site, config string) (string, error) {
	// 确定配置文件路径
	var configPath string
	if site.ConfigExportPath != "" {
		configPath = site.ConfigExportPath
	} else {
		configPath = filepath.Join(g.configDir, fmt.Sprintf("%s.conf", site.Domain))
	}

	// 确保目录存在
	configDir := filepath.Dir(configPath)
	if err := os.MkdirAll(configDir, 0755); err != nil {
		return "", fmt.Errorf("create config dir failed: %w", err)
	}

	// 写入配置文件
	if err := os.WriteFile(configPath, []byte(config), 0644); err != nil {
		return "", fmt.Errorf("write config file failed: %w", err)
	}

	return configPath, nil
}

// GenerateAndSave 生成并保存配置
func (g *Generator) GenerateAndSave(site *models.Site, ipBlacklist []string) (string, error) {
	// 生成配置
	config, err := g.GenerateSiteConfig(site, ipBlacklist)
	if err != nil {
		return "", fmt.Errorf("generate config failed: %w", err)
	}

	// 保存配置
	configPath, err := g.SaveSiteConfig(site, config)
	if err != nil {
		return "", fmt.Errorf("save config failed: %w", err)
	}

	return configPath, nil
}

// TestConfig 测试 Nginx 配置
func (g *Generator) TestConfig() error {
	cmd := exec.Command("sh", "-c", g.config.TestCmd)
	output, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("nginx config test failed: %s", string(output))
	}
	return nil
}

// ReloadNginx 重载 Nginx
func (g *Generator) ReloadNginx() error {
	cmd := exec.Command("sh", "-c", g.config.ReloadCmd)
	output, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("nginx reload failed: %s", string(output))
	}
	logger.Info("nginx reloaded successfully")
	return nil
}

// DeleteSiteConfig 删除站点配置
func (g *Generator) DeleteSiteConfig(site *models.Site) error {
	var configPath string
	if site.ConfigExportPath != "" {
		configPath = site.ConfigExportPath
	} else {
		configPath = filepath.Join(g.configDir, fmt.Sprintf("%s.conf", site.Domain))
	}

	if err := os.Remove(configPath); err != nil && !os.IsNotExist(err) {
		return fmt.Errorf("delete config file failed: %w", err)
	}

	return nil
}

// GetSiteConfigContent 获取站点配置内容
func (g *Generator) GetSiteConfigContent(site *models.Site) (string, error) {
	var configPath string
	if site.ConfigExportPath != "" {
		configPath = site.ConfigExportPath
	} else {
		configPath = filepath.Join(g.configDir, fmt.Sprintf("%s.conf", site.Domain))
	}

	content, err := os.ReadFile(configPath)
	if err != nil {
		if os.IsNotExist(err) {
			return "", nil
		}
		return "", fmt.Errorf("read config file failed: %w", err)
	}

	return string(content), nil
}

// GenerateMainConfig 生成主配置文件
func (g *Generator) GenerateMainConfig(sites []*models.Site) (string, error) {
	var includes []string
	for _, site := range sites {
		if site.IsActive() {
			configFile := fmt.Sprintf("%s.conf", site.Domain)
			includes = append(includes, configFile)
		}
	}

	mainConfig := `# Senix Nginx Main Configuration
# Auto-generated, do not edit manually

user nginx;
worker_processes auto;
error_log /var/log/nginx/error.log warn;
pid /var/run/nginx.pid;

events {
    worker_connections 1024;
    use epoll;
    multi_accept on;
}

http {
    include /etc/nginx/mime.types;
    default_type application/octet-stream;

    # Logging format
    log_format main '$remote_addr - $remote_user [$time_local] "$request" '
                    '$status $body_bytes_sent "$http_referer" '
                    '"$http_user_agent" "$http_x_forwarded_for"';

    access_log /var/log/nginx/access.log main;

    # Performance
    sendfile on;
    tcp_nopush on;
    tcp_nodelay on;
    keepalive_timeout 65;
    types_hash_max_size 2048;

    # Gzip
    gzip on;
    gzip_vary on;
    gzip_proxied any;
    gzip_comp_level 6;
    gzip_types text/plain text/css text/xml application/json application/javascript application/rss+xml application/atom+xml image/svg+xml;

    # Virtual Host Configs
`

	for _, include := range includes {
		mainConfig += fmt.Sprintf("    include %s;\n", filepath.Join(g.configDir, include))
	}

	mainConfig += "}\n"

	return mainConfig, nil
}

// ValidateUpstream 验证 upstream 格式
func ValidateUpstream(upstream string) error {
	if upstream == "" {
		return nil
	}

	// 支持格式:
	// - http://backend:8080
	// - https://backend:443
	// - backend:8080 (自动添加 http://)

	// 如果不包含协议，添加 http://
	if !strings.HasPrefix(upstream, "http://") && !strings.HasPrefix(upstream, "https://") {
		upstream = "http://" + upstream
	}

	// 简单验证格式
	if !strings.Contains(upstream, ":") {
		return fmt.Errorf("upstream must contain port number")
	}

	return nil
}
