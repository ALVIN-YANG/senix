package site

import (
	"encoding/json"
	"fmt"

	"senix/internal/cert"
	"senix/internal/config"
	"senix/internal/database"
	"senix/internal/logger"
	"senix/internal/models"
	"senix/internal/nginx"
)

// Service 站点服务
type Service struct {
	nginxGen      *nginx.Generator
	certService   *cert.Service
	config        *config.Config
}

// NewService 创建站点服务
func NewService(cfg *config.Config) *Service {
	return &Service{
		nginxGen:    nginx.NewGenerator(&cfg.Nginx),
		certService: cert.NewService(&cfg.Cert, cfg.Cert.DataDir),
		config:      cfg,
	}
}

// Initialize 初始化服务
func (s *Service) Initialize() error {
	// 初始化 Nginx 生成器
	if err := s.nginxGen.Initialize(); err != nil {
		return fmt.Errorf("init nginx generator failed: %w", err)
	}

	// 初始化证书服务
	if err := s.certService.Initialize(); err != nil {
		return fmt.Errorf("init cert service failed: %w", err)
	}

	return nil
}

// CreateSite 创建站点
func (s *Service) CreateSite(req *models.SiteRequest) (*models.Site, error) {
	// 验证 upstream
	if req.Upstream != "" {
		if err := nginx.ValidateUpstream(req.Upstream); err != nil {
			return nil, fmt.Errorf("invalid upstream: %w", err)
		}
	}

	// 创建站点
	site := &models.Site{
		Name:              req.Name,
		Domain:            req.Domain,
		WorkMode:          req.WorkMode,
		Port:              req.Port,
		SSLEnabled:        req.SSLEnabled,
		SSLAuto:           req.SSLAuto,
		Upstream:          req.Upstream,
		LoadBalance:       req.LoadBalance,
		CertID:            req.CertID,
		EnableWAF:         req.EnableWAF,
		EnableRateLimit:   req.EnableRateLimit,
		EnableIPBlacklist: req.EnableIPBlacklist,
		WAFMode:           req.WAFMode,
		RateLimitReqPS:    req.RateLimitReqPS,
		RateLimitBurst:    req.RateLimitBurst,
		CertExportPath:    req.CertExportPath,
		AutoReloadNginx:   req.AutoReloadNginx,
		NginxReloadCmd:    req.NginxReloadCmd,
		ConfigExportPath:  req.ConfigExportPath,
		NginxTemplate:     req.NginxTemplate,
		Description:       req.Description,
		Status:            "inactive",
	}

	// 如果启用 SSL 且指定了证书，验证证书存在
	if site.SSLEnabled && site.CertID != nil {
		_, err := s.certService.GetCertificate(*site.CertID)
		if err != nil {
			return nil, fmt.Errorf("certificate not found: %w", err)
		}
	}

	// 保存到数据库
	if err := database.DB.Create(site).Error; err != nil {
		return nil, fmt.Errorf("create site failed: %w", err)
	}

	// 预加载证书信息
	if site.CertID != nil {
		database.DB.Preload("Cert").First(site, site.ID)
	}

	return site, nil
}

// GetSite 获取站点
func (s *Service) GetSite(id uint) (*models.Site, error) {
	var site models.Site
	if err := database.DB.Preload("Cert").First(&site, id).Error; err != nil {
		return nil, fmt.Errorf("site not found: %w", err)
	}
	return &site, nil
}

// GetSiteByDomain 通过域名获取站点
func (s *Service) GetSiteByDomain(domain string) (*models.Site, error) {
	var site models.Site
	if err := database.DB.Preload("Cert").Where("domain = ?", domain).First(&site).Error; err != nil {
		return nil, fmt.Errorf("site not found: %w", err)
	}
	return &site, nil
}

// ListSites 列出所有站点
func (s *Service) ListSites(page, pageSize int) ([]*models.Site, int64, error) {
	var sites []*models.Site
	var total int64

	// 获取总数
	if err := database.DB.Model(&models.Site{}).Count(&total).Error; err != nil {
		return nil, 0, err
	}

	// 分页查询
	offset := (page - 1) * pageSize
	if err := database.DB.Preload("Cert").Order("created_at DESC").Offset(offset).Limit(pageSize).Find(&sites).Error; err != nil {
		return nil, 0, err
	}

	return sites, total, nil
}

// UpdateSite 更新站点
func (s *Service) UpdateSite(id uint, req *models.SiteRequest) (*models.Site, error) {
	site, err := s.GetSite(id)
	if err != nil {
		return nil, err
	}

	// 验证 upstream
	if req.Upstream != "" {
		if err := nginx.ValidateUpstream(req.Upstream); err != nil {
			return nil, fmt.Errorf("invalid upstream: %w", err)
		}
	}

	// 更新字段
	site.Name = req.Name
	site.Domain = req.Domain
	site.WorkMode = req.WorkMode
	site.Port = req.Port
	site.SSLEnabled = req.SSLEnabled
	site.SSLAuto = req.SSLAuto
	site.Upstream = req.Upstream
	site.LoadBalance = req.LoadBalance
	site.CertID = req.CertID
	site.EnableWAF = req.EnableWAF
	site.EnableRateLimit = req.EnableRateLimit
	site.EnableIPBlacklist = req.EnableIPBlacklist
	site.WAFMode = req.WAFMode
	site.RateLimitReqPS = req.RateLimitReqPS
	site.RateLimitBurst = req.RateLimitBurst
	site.CertExportPath = req.CertExportPath
	site.AutoReloadNginx = req.AutoReloadNginx
	site.NginxReloadCmd = req.NginxReloadCmd
	site.ConfigExportPath = req.ConfigExportPath
	site.NginxTemplate = req.NginxTemplate
	site.Description = req.Description

	// 保存更新
	if err := database.DB.Save(site).Error; err != nil {
		return nil, fmt.Errorf("update site failed: %w", err)
	}

	// 重新加载证书信息
	database.DB.Preload("Cert").First(site, site.ID)

	return site, nil
}

// DeleteSite 删除站点
func (s *Service) DeleteSite(id uint) error {
	site, err := s.GetSite(id)
	if err != nil {
		return err
	}

	// 如果站点是激活状态，先禁用
	if site.IsActive() {
		if err := s.DisableSite(id); err != nil {
			logger.Error("disable site before delete failed", logger.ErrorField(err))
		}
	}

	// 删除配置文件
	if err := s.nginxGen.DeleteSiteConfig(site); err != nil {
		logger.Error("delete site config failed", logger.ErrorField(err))
	}

	// 从数据库删除
	return database.DB.Delete(site).Error
}

// EnableSite 启用站点
func (s *Service) EnableSite(id uint) (*models.Site, error) {
	site, err := s.GetSite(id)
	if err != nil {
		return nil, err
	}

	// 根据工作模式处理
	switch site.WorkMode {
	case models.WorkModeStandalone:
		// 独立代理模式：生成配置并启动
		if err := s.generateAndApplyConfig(site); err != nil {
			site.Status = "error"
			site.ErrorMsg = err.Error()
			database.DB.Save(site)
			return nil, err
		}

	case models.WorkModeCertOnly:
		// 证书管理模式：导出证书
		if err := s.exportCertificate(site); err != nil {
			site.Status = "error"
			site.ErrorMsg = err.Error()
			database.DB.Save(site)
			return nil, err
		}

	case models.WorkModeConfigOnly:
		// 配置生成模式：生成配置
		if err := s.generateConfigOnly(site); err != nil {
			site.Status = "error"
			site.ErrorMsg = err.Error()
			database.DB.Save(site)
			return nil, err
		}
	}

	site.Status = "active"
	site.ErrorMsg = ""
	if err := database.DB.Save(site).Error; err != nil {
		return nil, err
	}

	return site, nil
}

// DisableSite 禁用站点
func (s *Service) DisableSite(id uint) (*models.Site, error) {
	site, err := s.GetSite(id)
	if err != nil {
		return nil, err
	}

	// 删除配置文件
	if err := s.nginxGen.DeleteSiteConfig(site); err != nil {
		logger.Error("delete site config failed", logger.ErrorField(err))
	}

	site.Status = "inactive"
	site.ErrorMsg = ""
	if err := database.DB.Save(site).Error; err != nil {
		return nil, err
	}

	return site, nil
}

// generateAndApplyConfig 生成并应用配置
func (s *Service) generateAndApplyConfig(site *models.Site) error {
	// 获取 IP 黑名单
	var ipBlacklist []string
	if site.EnableIPBlacklist {
		var blacklist []models.IPBlacklist
		if err := database.DB.Where("site_id = ? OR site_id IS NULL", site.ID).Find(&blacklist).Error; err == nil {
			for _, ip := range blacklist {
				if !ip.IsExpired() {
					ipBlacklist = append(ipBlacklist, ip.IP)
				}
			}
		}
	}

	// 生成并保存配置
	configPath, err := s.nginxGen.GenerateAndSave(site, ipBlacklist)
	if err != nil {
		return fmt.Errorf("generate config failed: %w", err)
	}

	logger.Info("site config generated", logger.String("path", configPath))

	// 测试配置
	if err := s.nginxGen.TestConfig(); err != nil {
		return fmt.Errorf("nginx config test failed: %w", err)
	}

	// 重载 Nginx
	if err := s.nginxGen.ReloadNginx(); err != nil {
		return fmt.Errorf("nginx reload failed: %w", err)
	}

	return nil
}

// exportCertificate 导出证书
func (s *Service) exportCertificate(site *models.Site) error {
	if site.CertID == nil {
		return fmt.Errorf("no certificate associated")
	}

	exportPath := site.CertExportPath
	if exportPath == "" {
		exportPath = s.config.Nginx.CertDir
	}

	if err := s.certService.ExportToPath(*site.CertID, exportPath); err != nil {
		return fmt.Errorf("export certificate failed: %w", err)
	}

	logger.Info("certificate exported", logger.String("domain", site.Domain), logger.String("path", exportPath))

	// 自动重载 Nginx
	if site.AutoReloadNginx {
		if err := s.nginxGen.ReloadNginx(); err != nil {
			logger.Error("auto reload nginx failed", logger.ErrorField(err))
		}
	}

	return nil
}

// generateConfigOnly 仅生成配置
func (s *Service) generateConfigOnly(site *models.Site) error {
	// 获取 IP 黑名单
	var ipBlacklist []string
	if site.EnableIPBlacklist {
		var blacklist []models.IPBlacklist
		if err := database.DB.Where("site_id = ? OR site_id IS NULL", site.ID).Find(&blacklist).Error; err == nil {
			for _, ip := range blacklist {
				if !ip.IsExpired() {
					ipBlacklist = append(ipBlacklist, ip.IP)
				}
			}
		}
	}

	// 生成并保存配置
	configPath, err := s.nginxGen.GenerateAndSave(site, ipBlacklist)
	if err != nil {
		return fmt.Errorf("generate config failed: %w", err)
	}

	logger.Info("site config generated", logger.String("path", configPath))

	return nil
}

// GetSiteConfig 获取站点配置内容
func (s *Service) GetSiteConfig(id uint) (string, error) {
	site, err := s.GetSite(id)
	if err != nil {
		return "", err
	}

	return s.nginxGen.GetSiteConfigContent(site)
}

// GetSiteStats 获取站点统计
func (s *Service) GetSiteStats() (*models.SiteStats, error) {
	stats := &models.SiteStats{}

	if err := database.DB.Model(&models.Site{}).Count(&stats.Total).Error; err != nil {
		return nil, err
	}

	if err := database.DB.Model(&models.Site{}).Where("status = ?", "active").Count(&stats.Active).Error; err != nil {
		return nil, err
	}

	if err := database.DB.Model(&models.Site{}).Where("status = ?", "inactive").Count(&stats.Inactive).Error; err != nil {
		return nil, err
	}

	if err := database.DB.Model(&models.Site{}).Where("ssl_enabled = ?", true).Count(&stats.WithSSL).Error; err != nil {
		return nil, err
	}

	if err := database.DB.Model(&models.Site{}).Where("work_mode = ?", models.WorkModeStandalone).Count(&stats.Standalone).Error; err != nil {
		return nil, err
	}

	if err := database.DB.Model(&models.Site{}).Where("work_mode = ?", models.WorkModeCertOnly).Count(&stats.CertOnly).Error; err != nil {
		return nil, err
	}

	if err := database.DB.Model(&models.Site{}).Where("work_mode = ?", models.WorkModeConfigOnly).Count(&stats.ConfigOnly).Error; err != nil {
		return nil, err
	}

	return stats, nil
}

// GenerateAllConfigs 生成所有激活站点的配置
func (s *Service) GenerateAllConfigs() error {
	var sites []*models.Site
	if err := database.DB.Preload("Cert").Where("status = ?", "active").Find(&sites).Error; err != nil {
		return err
	}

	for _, site := range sites {
		if err := s.generateAndApplyConfig(site); err != nil {
			logger.Error("generate config for site failed",
				logger.String("domain", site.Domain),
				logger.ErrorField(err))
		}
	}

	return nil
}

// ImportSites 批量导入站点
func (s *Service) ImportSites(sitesData []byte) error {
	var sites []models.SiteRequest
	if err := json.Unmarshal(sitesData, &sites); err != nil {
		return fmt.Errorf("parse sites data failed: %w", err)
	}

	for _, req := range sites {
		if _, err := s.CreateSite(&req); err != nil {
			logger.Error("create site failed",
				logger.String("domain", req.Domain),
				logger.ErrorField(err))
		}
	}

	return nil
}

// ExportSites 导出所有站点
func (s *Service) ExportSites() ([]byte, error) {
	var sites []*models.Site
	if err := database.DB.Find(&sites).Error; err != nil {
		return nil, err
	}

	var requests []models.SiteRequest
	for _, site := range sites {
		req := models.SiteRequest{
			Name:              site.Name,
			Domain:            site.Domain,
			WorkMode:          site.WorkMode,
			Port:              site.Port,
			SSLEnabled:        site.SSLEnabled,
			SSLAuto:           site.SSLAuto,
			Upstream:          site.Upstream,
			LoadBalance:       site.LoadBalance,
			CertID:            site.CertID,
			EnableWAF:         site.EnableWAF,
			EnableRateLimit:   site.EnableRateLimit,
			EnableIPBlacklist: site.EnableIPBlacklist,
			WAFMode:           site.WAFMode,
			RateLimitReqPS:    site.RateLimitReqPS,
			RateLimitBurst:    site.RateLimitBurst,
			Description:       site.Description,
		}
		requests = append(requests, req)
	}

	return json.MarshalIndent(requests, "", "  ")
}
