package handler

import (
	"net/http"
	"strconv"

	"github.com/gin-gonic/gin"

	"senix/internal/cert"
	"senix/internal/config"
	"senix/internal/logger"
	"senix/internal/models"
	"senix/pkg/utils"
)

// CertHandler 证书处理器
type CertHandler struct {
	service *cert.Service
}

// NewCertHandler 创建证书处理器
func NewCertHandler(cfg *config.Config) *CertHandler {
	service := cert.NewService(&cfg.Cert, cfg.Cert.DataDir)
	return &CertHandler{
		service: service,
	}
}

// Initialize 初始化
func (h *CertHandler) Initialize() error {
	return h.service.Initialize()
}

// CreateACME 申请 ACME 证书
func (h *CertHandler) CreateACME(c *gin.Context) {
	var req models.ACMERequest
	if err := c.ShouldBindJSON(&req); err != nil {
		utils.Error(c, http.StatusBadRequest, "Invalid request", err.Error())
		return
	}

	cert, err := h.service.CreateACME(&req)
	if err != nil {
		logger.Error("create acme certificate failed", logger.ErrorField(err))
		utils.Error(c, http.StatusInternalServerError, "Failed to create certificate", err.Error())
		return
	}

	utils.Success(c, cert.ToResponse())
}

// UploadCertificate 上传证书
func (h *CertHandler) UploadCertificate(c *gin.Context) {
	domain := c.PostForm("domain")
	if domain == "" {
		utils.Error(c, http.StatusBadRequest, "Invalid request", "domain is required")
		return
	}

	// 获取证书文件
	certFile, err := c.FormFile("cert")
	if err != nil {
		utils.Error(c, http.StatusBadRequest, "Invalid request", "certificate file is required")
		return
	}

	// 获取私钥文件
	keyFile, err := c.FormFile("key")
	if err != nil {
		utils.Error(c, http.StatusBadRequest, "Invalid request", "key file is required")
		return
	}

	// 读取证书内容
	certFileContent, err := certFile.Open()
	if err != nil {
		utils.Error(c, http.StatusInternalServerError, "Failed to read certificate", err.Error())
		return
	}
	defer certFileContent.Close()

	certPEM := make([]byte, certFile.Size)
	if _, err := certFileContent.Read(certPEM); err != nil {
		utils.Error(c, http.StatusInternalServerError, "Failed to read certificate", err.Error())
		return
	}

	// 读取私钥内容
	keyFileContent, err := keyFile.Open()
	if err != nil {
		utils.Error(c, http.StatusInternalServerError, "Failed to read key", err.Error())
		return
	}
	defer keyFileContent.Close()

	keyPEM := make([]byte, keyFile.Size)
	if _, err := keyFileContent.Read(keyPEM); err != nil {
		utils.Error(c, http.StatusInternalServerError, "Failed to read key", err.Error())
		return
	}

	// 上传证书
	cert, err := h.service.UploadCertificate(domain, certPEM, keyPEM)
	if err != nil {
		logger.Error("upload certificate failed", logger.ErrorField(err))
		utils.Error(c, http.StatusInternalServerError, "Failed to upload certificate", err.Error())
		return
	}

	utils.Success(c, cert.ToResponse())
}

// GetCertificates 获取证书列表
func (h *CertHandler) GetCertificates(c *gin.Context) {
	page, _ := strconv.Atoi(c.DefaultQuery("page", "1"))
	pageSize, _ := strconv.Atoi(c.DefaultQuery("page_size", "10"))

	if page < 1 {
		page = 1
	}
	if pageSize < 1 || pageSize > 100 {
		pageSize = 10
	}

	certs, total, err := h.service.ListCertificates(page, pageSize)
	if err != nil {
		logger.Error("list certificates failed", logger.ErrorField(err))
		utils.Error(c, http.StatusInternalServerError, "Failed to list certificates", err.Error())
		return
	}

	// 转换为响应格式
	responses := make([]*models.CertificateResponse, len(certs))
	for i, cert := range certs {
		responses[i] = cert.ToResponse()
	}

	utils.Success(c, gin.H{
		"list":  responses,
		"total": total,
		"page":  page,
		"size":  pageSize,
	})
}

// GetCertificate 获取单个证书
func (h *CertHandler) GetCertificate(c *gin.Context) {
	id, err := strconv.ParseUint(c.Param("id"), 10, 32)
	if err != nil {
		utils.Error(c, http.StatusBadRequest, "Invalid request", "invalid certificate id")
		return
	}

	cert, err := h.service.GetCertificate(uint(id))
	if err != nil {
		utils.Error(c, http.StatusNotFound, "Certificate not found", err.Error())
		return
	}

	utils.Success(c, cert.ToResponse())
}

// DeleteCertificate 删除证书
func (h *CertHandler) DeleteCertificate(c *gin.Context) {
	id, err := strconv.ParseUint(c.Param("id"), 10, 32)
	if err != nil {
		utils.Error(c, http.StatusBadRequest, "Invalid request", "invalid certificate id")
		return
	}

	if err := h.service.DeleteCertificate(uint(id)); err != nil {
		logger.Error("delete certificate failed", logger.ErrorField(err))
		utils.Error(c, http.StatusInternalServerError, "Failed to delete certificate", err.Error())
		return
	}

	utils.Success(c, gin.H{"message": "certificate deleted"})
}

// RenewCertificate 续期证书
func (h *CertHandler) RenewCertificate(c *gin.Context) {
	id, err := strconv.ParseUint(c.Param("id"), 10, 32)
	if err != nil {
		utils.Error(c, http.StatusBadRequest, "Invalid request", "invalid certificate id")
		return
	}

	cert, err := h.service.RenewCertificate(uint(id))
	if err != nil {
		logger.Error("renew certificate failed", logger.ErrorField(err))
		utils.Error(c, http.StatusInternalServerError, "Failed to renew certificate", err.Error())
		return
	}

	utils.Success(c, cert.ToResponse())
}

// DownloadCertificate 下载证书
func (h *CertHandler) DownloadCertificate(c *gin.Context) {
	id, err := strconv.ParseUint(c.Param("id"), 10, 32)
	if err != nil {
		utils.Error(c, http.StatusBadRequest, "Invalid request", "invalid certificate id")
		return
	}

	format := c.DefaultQuery("format", "pem")

	certData, keyData, err := h.service.ExportCertificate(uint(id), format)
	if err != nil {
		utils.Error(c, http.StatusNotFound, "Certificate not found", err.Error())
		return
	}

	// 返回证书和私钥
	c.JSON(http.StatusOK, gin.H{
		"certificate": string(certData),
		"key":         string(keyData),
	})
}

// GetCertificateStats 获取证书统计
func (h *CertHandler) GetCertificateStats(c *gin.Context) {
	stats, err := h.service.GetCertificateStats()
	if err != nil {
		logger.Error("get certificate stats failed", logger.ErrorField(err))
		utils.Error(c, http.StatusInternalServerError, "Failed to get stats", err.Error())
		return
	}

	utils.Success(c, stats)
}

// GetDNSProviders 获取支持的 DNS 提供商列表
func (h *CertHandler) GetDNSProviders(c *gin.Context) {
	providers := []gin.H{
		{
			"name":        "cloudflare",
			"display_name": "Cloudflare",
			"required_fields": []gin.H{
				{"name": "api_token", "label": "API Token", "type": "password"},
			},
		},
		{
			"name":        "alidns",
			"display_name": "阿里云 DNS",
			"required_fields": []gin.H{
				{"name": "access_key", "label": "Access Key", "type": "text"},
				{"name": "secret_key", "label": "Secret Key", "type": "password"},
			},
		},
		{
			"name":        "tencentcloud",
			"display_name": "腾讯云 DNSPod",
			"required_fields": []gin.H{
				{"name": "secret_id", "label": "Secret ID", "type": "text"},
				{"name": "secret_key", "label": "Secret Key", "type": "password"},
			},
		},
	}

	utils.Success(c, providers)
}
