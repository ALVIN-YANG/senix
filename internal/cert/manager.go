package cert

import (
	"crypto"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"encoding/pem"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/go-acme/lego/v4/certcrypto"
	"github.com/go-acme/lego/v4/certificate"
	"github.com/go-acme/lego/v4/challenge"
	"github.com/go-acme/lego/v4/challenge/dns01"
	"github.com/go-acme/lego/v4/challenge/http01"
	"github.com/go-acme/lego/v4/lego"
	"github.com/go-acme/lego/v4/providers/dns/alidns"
	"github.com/go-acme/lego/v4/providers/dns/cloudflare"
	"github.com/go-acme/lego/v4/providers/dns/tencentcloud"
	"github.com/go-acme/lego/v4/registration"

	"senix/internal/config"
	"senix/internal/logger"
	"senix/internal/models"
)

// Manager 证书管理器
type Manager struct {
	config    *config.CertConfig
	dataDir   string
	user      *ACMEUser
}

// ACMEUser ACME 用户
type ACMEUser struct {
	Email        string
	Registration *registration.Resource
	key          crypto.PrivateKey
}

// GetEmail 获取邮箱
func (u *ACMEUser) GetEmail() string {
	return u.Email
}

// GetRegistration 获取注册信息
func (u *ACMEUser) GetRegistration() *registration.Resource {
	return u.Registration
}

// GetPrivateKey 获取私钥
func (u *ACMEUser) GetPrivateKey() crypto.PrivateKey {
	return u.key
}

// NewManager 创建证书管理器
func NewManager(cfg *config.CertConfig, dataDir string) *Manager {
	return &Manager{
		config:  cfg,
		dataDir: dataDir,
	}
}

// Initialize 初始化
func (m *Manager) Initialize() error {
	// 确保证书目录存在
	certDir := filepath.Join(m.dataDir, "certs")
	if err := os.MkdirAll(certDir, 0755); err != nil {
		return fmt.Errorf("create cert dir failed: %w", err)
	}

	return nil
}

// CreateACMEUser 创建 ACME 用户
func (m *Manager) CreateACMEUser(email string) (*ACMEUser, error) {
	// 生成私钥
	privateKey, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return nil, fmt.Errorf("generate private key failed: %w", err)
	}

	user := &ACMEUser{
		Email: email,
		key:   privateKey,
	}

	return user, nil
}

// RegisterACME 注册 ACME 账户
func (m *Manager) RegisterACME(user *ACMEUser, caDirURL string) error {
	config := lego.NewConfig(user)

	// 设置 CA 服务器
	if caDirURL != "" {
		config.CADirURL = caDirURL
	} else {
		config.CADirURL = lego.LEDirectoryProduction
	}

	// 设置密钥类型
	config.Certificate.KeyType = certcrypto.RSA2048

	// 创建客户端
	client, err := lego.NewClient(config)
	if err != nil {
		return fmt.Errorf("create lego client failed: %w", err)
	}

	// 注册账户
	reg, err := client.Registration.Register(registration.RegisterOptions{TermsOfServiceAgreed: true})
	if err != nil {
		return fmt.Errorf("register acme account failed: %w", err)
	}

	user.Registration = reg
	return nil
}

// ObtainCertificate 申请证书
func (m *Manager) ObtainCertificate(user *ACMEUser, domains []string, challengeType string, providerConfig map[string]string) (*certificate.Resource, error) {
	config := lego.NewConfig(user)
	config.CADirURL = lego.LEDirectoryProduction
	config.Certificate.KeyType = certcrypto.RSA2048

	client, err := lego.NewClient(config)
	if err != nil {
		return nil, fmt.Errorf("create lego client failed: %w", err)
	}

	// 设置挑战提供者
	switch challengeType {
	case "http-01":
		// HTTP-01 挑战需要配合 Web 服务器
		client.Challenge.SetHTTP01Provider(http01.NewProviderServer("", "80"))
	case "dns-01":
		provider, err := m.createDNSProvider(providerConfig)
		if err != nil {
			return nil, fmt.Errorf("create dns provider failed: %w", err)
		}
		client.Challenge.Remove(challenge.HTTP01)
		client.Challenge.Remove(challenge.TLSALPN01)
		client.Challenge.AddDNS01(provider)
	default:
		return nil, fmt.Errorf("unsupported challenge type: %s", challengeType)
	}

	// 申请证书
	request := certificate.ObtainRequest{
		Domains: domains,
		Bundle:  true,
	}

	certificates, err := client.Certificate.Obtain(request)
	if err != nil {
		return nil, fmt.Errorf("obtain certificate failed: %w", err)
	}

	return certificates, nil
}

// createDNSProvider 创建 DNS 提供者
func (m *Manager) createDNSProvider(config map[string]string) (challenge.Provider, error) {
	provider := config["provider"]
	if provider == "" {
		provider = "cloudflare"
	}

	switch provider {
	case "cloudflare":
		return cloudflare.NewDNSProviderConfig(&cloudflare.Config{
			AuthEmail:          config["email"],
			AuthKey:            config["api_key"],
			AuthToken:          config["api_token"],
			PropagationTimeout: time.Minute * 2,
			PollingInterval:    time.Second * 2,
		})
	case "alidns", "alibaba":
		return alidns.NewDNSProviderConfig(&alidns.Config{
			APIKey:             config["access_key"],
			SecretKey:          config["secret_key"],
			PropagationTimeout: time.Minute * 2,
			PollingInterval:    time.Second * 2,
		})
	case "tencentcloud", "dnspod":
		return tencentcloud.NewDNSProviderConfig(&tencentcloud.Config{
			SecretID:           config["secret_id"],
			SecretKey:          config["secret_key"],
			PropagationTimeout: time.Minute * 2,
			PollingInterval:    time.Second * 2,
		})
	default:
		return nil, fmt.Errorf("unsupported dns provider: %s", provider)
	}
}

// SaveCertificate 保存证书到文件
func (m *Manager) SaveCertificate(domain string, cert *certificate.Resource) (certPath, keyPath, chainPath string, err error) {
	certDir := filepath.Join(m.dataDir, "certs", domain)
	if err := os.MkdirAll(certDir, 0755); err != nil {
		return "", "", "", fmt.Errorf("create cert dir failed: %w", err)
	}

	certPath = filepath.Join(certDir, "cert.pem")
	keyPath = filepath.Join(certDir, "key.pem")
	chainPath = filepath.Join(certDir, "chain.pem")

	// 保存证书
	if err := os.WriteFile(certPath, cert.Certificate, 0644); err != nil {
		return "", "", "", fmt.Errorf("save certificate failed: %w", err)
	}

	// 保存私钥
	if err := os.WriteFile(keyPath, cert.PrivateKey, 0600); err != nil {
		return "", "", "", fmt.Errorf("save private key failed: %w", err)
	}

	// 保存证书链
	if err := os.WriteFile(chainPath, cert.IssuerCertificate, 0644); err != nil {
		return "", "", "", fmt.Errorf("save chain failed: %w", err)
	}

	return certPath, keyPath, chainPath, nil
}

// LoadCertificate 从文件加载证书
func (m *Manager) LoadCertificate(domain string) (*certificate.Resource, error) {
	certDir := filepath.Join(m.dataDir, "certs", domain)

	certPath := filepath.Join(certDir, "cert.pem")
	keyPath := filepath.Join(certDir, "key.pem")
	chainPath := filepath.Join(certDir, "chain.pem")

	certPEM, err := os.ReadFile(certPath)
	if err != nil {
		return nil, fmt.Errorf("read certificate failed: %w", err)
	}

	keyPEM, err := os.ReadFile(keyPath)
	if err != nil {
		return nil, fmt.Errorf("read private key failed: %w", err)
	}

	chainPEM, err := os.ReadFile(chainPath)
	if err != nil {
		return nil, fmt.Errorf("read chain failed: %w", err)
	}

	return &certificate.Resource{
		Domain:            domain,
		Certificate:       certPEM,
		PrivateKey:        keyPEM,
		IssuerCertificate: chainPEM,
	}, nil
}

// ParseCertificate 解析证书信息
func (m *Manager) ParseCertificate(certPEM []byte) (*x509.Certificate, error) {
	block, _ := pem.Decode(certPEM)
	if block == nil {
		return nil, fmt.Errorf("failed to parse certificate PEM")
	}

	cert, err := x509.ParseCertificate(block.Bytes)
	if err != nil {
		return nil, fmt.Errorf("failed to parse certificate: %w", err)
	}

	return cert, nil
}

// GetCertificateInfo 获取证书信息
func (m *Manager) GetCertificateInfo(cert *x509.Certificate) *models.CertificateInfo {
	return &models.CertificateInfo{
		Issuer:       cert.Issuer.CommonName,
		Subject:      cert.Subject.CommonName,
		NotBefore:    cert.NotBefore,
		NotAfter:     cert.NotAfter,
		SerialNumber: cert.SerialNumber.String(),
		SANs:         cert.DNSNames,
	}
}

// IsCertificateExpiring 检查证书是否即将过期
func (m *Manager) IsCertificateExpiring(cert *x509.Certificate, days int) bool {
	return time.Now().Add(time.Duration(days) * 24 * time.Hour).After(cert.NotAfter)
}

// ShouldRenew 判断是否需要续期
func (m *Manager) ShouldRenew(cert *models.Certificate) bool {
	if !cert.AutoRenew {
		return false
	}

	// 读取证书文件
	certPEM, err := os.ReadFile(cert.CertPath)
	if err != nil {
		logger.Error("read certificate for renewal check failed", logger.ErrorField(err))
		return false
	}

	// 解析证书
	x509Cert, err := m.ParseCertificate(certPEM)
	if err != nil {
		logger.Error("parse certificate for renewal check failed", logger.ErrorField(err))
		return false
	}

	return m.IsCertificateExpiring(x509Cert, m.config.RenewBeforeDays)
}

// RenewCertificate 续期证书
func (m *Manager) RenewCertificate(user *ACMEUser, cert *models.Certificate, challengeType string, providerConfig map[string]string) error {
	// 重新申请证书
	domains := []string{cert.Domain}
	if cert.SANs != "" {
		// 解析 SANs
		sans := strings.Split(cert.SANs, ",")
		for _, san := range sans {
			san = strings.TrimSpace(san)
			if san != "" && san != cert.Domain {
				domains = append(domains, san)
			}
		}
	}

	newCert, err := m.ObtainCertificate(user, domains, challengeType, providerConfig)
	if err != nil {
		return fmt.Errorf("renew certificate failed: %w", err)
	}

	// 保存新证书
	certPath, keyPath, chainPath, err := m.SaveCertificate(cert.Domain, newCert)
	if err != nil {
		return fmt.Errorf("save renewed certificate failed: %w", err)
	}

	// 更新证书信息
	cert.CertPath = certPath
	cert.KeyPath = keyPath
	cert.ChainPath = chainPath

	// 解析证书信息
	x509Cert, err := m.ParseCertificate(newCert.Certificate)
	if err != nil {
		return fmt.Errorf("parse renewed certificate failed: %w", err)
	}

	info := m.GetCertificateInfo(x509Cert)
	cert.Issuer = info.Issuer
	cert.Subject = info.Subject
	cert.NotBefore = info.NotBefore
	cert.NotAfter = info.NotAfter
	cert.SerialNumber = info.SerialNumber

	now := time.Now()
	cert.RenewedAt = &now
	cert.Status = "active"

	return nil
}

// UploadCertificate 上传证书
func (m *Manager) UploadCertificate(domain string, certPEM, keyPEM []byte) (*models.Certificate, error) {
	// 验证证书
	x509Cert, err := m.ParseCertificate(certPEM)
	if err != nil {
		return nil, fmt.Errorf("parse certificate failed: %w", err)
	}

	// 验证私钥
	_, err = parsePrivateKey(keyPEM)
	if err != nil {
		return nil, fmt.Errorf("parse private key failed: %w", err)
	}

	// 保存证书
	certDir := filepath.Join(m.dataDir, "certs", domain)
	if err := os.MkdirAll(certDir, 0755); err != nil {
		return nil, fmt.Errorf("create cert dir failed: %w", err)
	}

	certPath := filepath.Join(certDir, "cert.pem")
	keyPath := filepath.Join(certDir, "key.pem")

	if err := os.WriteFile(certPath, certPEM, 0644); err != nil {
		return nil, fmt.Errorf("save certificate failed: %w", err)
	}

	if err := os.WriteFile(keyPath, keyPEM, 0600); err != nil {
		return nil, fmt.Errorf("save private key failed: %w", err)
	}

	// 创建证书记录
	info := m.GetCertificateInfo(x509Cert)
	cert := &models.Certificate{
		Domain:       domain,
		Source:       models.CertSourceUpload,
		CertPath:     certPath,
		KeyPath:      keyPath,
		Issuer:       info.Issuer,
		Subject:      info.Subject,
		NotBefore:    info.NotBefore,
		NotAfter:     info.NotAfter,
		SerialNumber: info.SerialNumber,
		Status:       "active",
	}

	return cert, nil
}

// parsePrivateKey 解析私钥
func parsePrivateKey(keyPEM []byte) (crypto.PrivateKey, error) {
	block, _ := pem.Decode(keyPEM)
	if block == nil {
		return nil, fmt.Errorf("failed to parse key PEM")
	}

	switch block.Type {
	case "RSA PRIVATE KEY":
		return x509.ParsePKCS1PrivateKey(block.Bytes)
	case "EC PRIVATE KEY":
		return x509.ParseECPrivateKey(block.Bytes)
	case "PRIVATE KEY":
		return x509.ParsePKCS8PrivateKey(block.Bytes)
	default:
		return nil, fmt.Errorf("unsupported key type: %s", block.Type)
	}
}
