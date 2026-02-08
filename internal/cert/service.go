package cert

import (
	"encoding/json"
	"fmt"
	"os"
	"time"

	"senix/internal/config"
	"senix/internal/database"
	"senix/internal/logger"
	"senix/internal/models"
)

// Service 证书服务
type Service struct {
	manager *Manager
	config  *config.CertConfig
}

// NewService 创建证书服务
func NewService(cfg *config.CertConfig, dataDir string) *Service {
	manager := NewManager(cfg, dataDir)
	return &Service{
		manager: manager,
		config:  cfg,
	}
}

// Initialize 初始化服务
func (s *Service) Initialize() error {
	return s.manager.Initialize()
}

// CreateACME 通过 ACME 申请证书
func (s *Service) CreateACME(req *models.ACMERequest) (*models.Certificate, error) {
	// 创建 ACME 用户
	user, err := s.manager.CreateACMEUser(req.Email)
	if err != nil {
		return nil, fmt.Errorf("create acme user failed: %w", err)
	}

	// 注册 ACME 账户
	if err := s.manager.RegisterACME(user, ""); err != nil {
		return nil, fmt.Errorf("register acme account failed: %w", err)
	}

	// 申请证书
	certResource, err := s.manager.ObtainCertificate(user, []string{req.Domain}, req.Challenge, req.DNSConfig)
	if err != nil {
		return nil, fmt.Errorf("obtain certificate failed: %w", err)
	}

	// 保存证书
	certPath, keyPath, chainPath, err := s.manager.SaveCertificate(req.Domain, certResource)
	if err != nil {
		return nil, fmt.Errorf("save certificate failed: %w", err)
	}

	// 解析证书信息
	x509Cert, err := s.manager.ParseCertificate(certResource.Certificate)
	if err != nil {
		return nil, fmt.Errorf("parse certificate failed: %w", err)
	}

	info := s.manager.GetCertificateInfo(x509Cert)

	// 序列化 DNS 配置
	dnsConfigJSON, _ := json.Marshal(req.DNSConfig)

	// 创建证书记录
	cert := &models.Certificate{
		Domain:      req.Domain,
		Source:      models.CertSourceACME,
		ACMEEmail:   req.Email,
		DNSProvider: req.DNSProvider,
		DNSConfig:   string(dnsConfigJSON),
		CertPath:    certPath,
		KeyPath:     keyPath,
		ChainPath:   chainPath,
		Issuer:      info.Issuer,
		Subject:     info.Subject,
		SANs:        joinStrings(info.SANs, ","),
		NotBefore:   info.NotBefore,
		NotAfter:    info.NotAfter,
		SerialNumber: info.SerialNumber,
		AutoRenew:   req.AutoRenew,
		Status:      "active",
	}

	// 保存到数据库
	if err := database.DB.Create(cert).Error; err != nil {
		return nil, fmt.Errorf("save certificate to database failed: %w", err)
	}

	return cert, nil
}

// UploadCertificate 上传证书
func (s *Service) UploadCertificate(domain string, certPEM, keyPEM []byte) (*models.Certificate, error) {
	// 上传并保存证书
	cert, err := s.manager.UploadCertificate(domain, certPEM, keyPEM)
	if err != nil {
		return nil, err
	}

	// 保存到数据库
	if err := database.DB.Create(cert).Error; err != nil {
		return nil, fmt.Errorf("save certificate to database failed: %w", err)
	}

	return cert, nil
}

// GetCertificate 获取证书
func (s *Service) GetCertificate(id uint) (*models.Certificate, error) {
	var cert models.Certificate
	if err := database.DB.First(&cert, id).Error; err != nil {
		return nil, fmt.Errorf("certificate not found: %w", err)
	}
	return &cert, nil
}

// GetCertificateByDomain 通过域名获取证书
func (s *Service) GetCertificateByDomain(domain string) (*models.Certificate, error) {
	var cert models.Certificate
	if err := database.DB.Where("domain = ?", domain).First(&cert).Error; err != nil {
		return nil, fmt.Errorf("certificate not found: %w", err)
	}
	return &cert, nil
}

// ListCertificates 列出所有证书
func (s *Service) ListCertificates(page, pageSize int) ([]*models.Certificate, int64, error) {
	var certs []*models.Certificate
	var total int64

	// 获取总数
	if err := database.DB.Model(&models.Certificate{}).Count(&total).Error; err != nil {
		return nil, 0, err
	}

	// 分页查询
	offset := (page - 1) * pageSize
	if err := database.DB.Order("created_at DESC").Offset(offset).Limit(pageSize).Find(&certs).Error; err != nil {
		return nil, 0, err
	}

	return certs, total, nil
}

// DeleteCertificate 删除证书
func (s *Service) DeleteCertificate(id uint) error {
	var cert models.Certificate
	if err := database.DB.First(&cert, id).Error; err != nil {
		return fmt.Errorf("certificate not found: %w", err)
	}

	// 删除证书文件
	if cert.CertPath != "" {
		_ = os.Remove(cert.CertPath)
	}
	if cert.KeyPath != "" {
		_ = os.Remove(cert.KeyPath)
	}
	if cert.ChainPath != "" {
		_ = os.Remove(cert.ChainPath)
	}

	// 从数据库删除
	return database.DB.Delete(&cert).Error
}

// RenewCertificate 续期证书
func (s *Service) RenewCertificate(id uint) (*models.Certificate, error) {
	var cert models.Certificate
	if err := database.DB.First(&cert, id).Error; err != nil {
		return nil, fmt.Errorf("certificate not found: %w", err)
	}

	// 只支持 ACME 证书续期
	if cert.Source != models.CertSourceACME {
		return nil, fmt.Errorf("only ACME certificates can be renewed")
	}

	// 解析 DNS 配置
	var dnsConfig map[string]string
	if cert.DNSConfig != "" {
		_ = json.Unmarshal([]byte(cert.DNSConfig), &dnsConfig)
	}

	// 确定挑战类型
	challengeType := "dns-01"
	if cert.DNSProvider == "" {
		challengeType = "http-01"
	}

	// 创建 ACME 用户
	user, err := s.manager.CreateACMEUser(cert.ACMEEmail)
	if err != nil {
		return nil, fmt.Errorf("create acme user failed: %w", err)
	}

	// 注册 ACME 账户
	if err := s.manager.RegisterACME(user, ""); err != nil {
		return nil, fmt.Errorf("register acme account failed: %w", err)
	}

	// 续期证书
	if err := s.manager.RenewCertificate(user, &cert, challengeType, dnsConfig); err != nil {
		cert.Status = "error"
		cert.ErrorMsg = err.Error()
		database.DB.Save(&cert)
		return nil, err
	}

	// 保存更新
	if err := database.DB.Save(&cert).Error; err != nil {
		return nil, fmt.Errorf("save renewed certificate failed: %w", err)
	}

	return &cert, nil
}

// CheckAndRenewCertificates 检查并续期即将过期的证书
func (s *Service) CheckAndRenewCertificates() error {
	var certs []models.Certificate
	if err := database.DB.Where("auto_renew = ? AND status = ?", true, "active").Find(&certs).Error; err != nil {
		return err
	}

	for _, cert := range certs {
		if s.manager.ShouldRenew(&cert) {
			logger.Info("renewing certificate", logger.String("domain", cert.Domain))
			if _, err := s.RenewCertificate(cert.ID); err != nil {
				logger.Error("renew certificate failed",
					logger.String("domain", cert.Domain),
					logger.ErrorField(err))
			}
		}
	}

	return nil
}

// ExportCertificate 导出证书
func (s *Service) ExportCertificate(id uint, format string) (certData, keyData []byte, err error) {
	cert, err := s.GetCertificate(id)
	if err != nil {
		return nil, nil, err
	}

	// 读取证书文件
	certData, err = os.ReadFile(cert.CertPath)
	if err != nil {
		return nil, nil, fmt.Errorf("read certificate file failed: %w", err)
	}

	// 读取私钥文件
	keyData, err = os.ReadFile(cert.KeyPath)
	if err != nil {
		return nil, nil, fmt.Errorf("read key file failed: %w", err)
	}

	return certData, keyData, nil
}

// ExportToPath 导出证书到指定路径
func (s *Service) ExportToPath(id uint, exportPath string) error {
	cert, err := s.GetCertificate(id)
	if err != nil {
		return err
	}

	// 读取证书和私钥
	certData, err := os.ReadFile(cert.CertPath)
	if err != nil {
		return fmt.Errorf("read certificate file failed: %w", err)
	}

	keyData, err := os.ReadFile(cert.KeyPath)
	if err != nil {
		return fmt.Errorf("read key file failed: %w", err)
	}

	// 确保目标目录存在
	if err := os.MkdirAll(exportPath, 0755); err != nil {
		return fmt.Errorf("create export directory failed: %w", err)
	}

	// 写入证书
	certExportPath := exportPath + "/" + cert.Domain + ".crt"
	if err := os.WriteFile(certExportPath, certData, 0644); err != nil {
		return fmt.Errorf("write certificate failed: %w", err)
	}

	// 写入私钥
	keyExportPath := exportPath + "/" + cert.Domain + ".key"
	if err := os.WriteFile(keyExportPath, keyData, 0600); err != nil {
		return fmt.Errorf("write key failed: %w", err)
	}

	return nil
}

// GetCertificateStats 获取证书统计信息
func (s *Service) GetCertificateStats() (map[string]interface{}, error) {
	var total int64
	var active int64
	var expired int64
	var expiringSoon int64

	if err := database.DB.Model(&models.Certificate{}).Count(&total).Error; err != nil {
		return nil, err
	}

	if err := database.DB.Model(&models.Certificate{}).Where("status = ?", "active").Count(&active).Error; err != nil {
		return nil, err
	}

	if err := database.DB.Model(&models.Certificate{}).Where("status = ?", "expired").Count(&expired).Error; err != nil {
		return nil, err
	}

	// 即将过期（30天内）
	soon := time.Now().AddDate(0, 0, 30)
	if err := database.DB.Model(&models.Certificate{}).Where("not_after < ? AND status = ?", soon, "active").Count(&expiringSoon).Error; err != nil {
		return nil, err
	}

	return map[string]interface{}{
		"total":         total,
		"active":        active,
		"expired":       expired,
		"expiring_soon": expiringSoon,
	}, nil
}

// joinStrings 将字符串数组用分隔符连接
func joinStrings(strs []string, sep string) string {
	if len(strs) == 0 {
		return ""
	}
	result := strs[0]
	for i := 1; i < len(strs); i++ {
		result += sep + strs[i]
	}
	return result
}
