package handler

import (
	"net/http"
	"strconv"

	"github.com/gin-gonic/gin"

	"senix/internal/config"
	"senix/internal/logger"
	"senix/internal/models"
	"senix/internal/site"
	"senix/pkg/utils"
)

// SiteHandler 站点处理器
type SiteHandler struct {
	service *site.Service
}

// NewSiteHandler 创建站点处理器
func NewSiteHandler(cfg *config.Config) *SiteHandler {
	service := site.NewService(cfg)
	return &SiteHandler{
		service: service,
	}
}

// Initialize 初始化
func (h *SiteHandler) Initialize() error {
	return h.service.Initialize()
}

// CreateSite 创建站点
func (h *SiteHandler) CreateSite(c *gin.Context) {
	var req models.SiteRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		utils.Error(c, http.StatusBadRequest, "Invalid request", err.Error())
		return
	}

	site, err := h.service.CreateSite(&req)
	if err != nil {
		logger.Error("create site failed", logger.ErrorField(err))
		utils.Error(c, http.StatusInternalServerError, "Failed to create site", err.Error())
		return
	}

	utils.Success(c, site.ToResponse())
}

// GetSites 获取站点列表
func (h *SiteHandler) GetSites(c *gin.Context) {
	page, _ := strconv.Atoi(c.DefaultQuery("page", "1"))
	pageSize, _ := strconv.Atoi(c.DefaultQuery("page_size", "10"))

	if page < 1 {
		page = 1
	}
	if pageSize < 1 || pageSize > 100 {
		pageSize = 10
	}

	sites, total, err := h.service.ListSites(page, pageSize)
	if err != nil {
		logger.Error("list sites failed", logger.ErrorField(err))
		utils.Error(c, http.StatusInternalServerError, "Failed to list sites", err.Error())
		return
	}

	// 转换为响应格式
	responses := make([]*models.SiteResponse, len(sites))
	for i, site := range sites {
		responses[i] = site.ToResponse()
	}

	utils.Success(c, gin.H{
		"list":  responses,
		"total": total,
		"page":  page,
		"size":  pageSize,
	})
}

// GetSite 获取单个站点
func (h *SiteHandler) GetSite(c *gin.Context) {
	id, err := strconv.ParseUint(c.Param("id"), 10, 32)
	if err != nil {
		utils.Error(c, http.StatusBadRequest, "Invalid request", "invalid site id")
		return
	}

	site, err := h.service.GetSite(uint(id))
	if err != nil {
		utils.Error(c, http.StatusNotFound, "Site not found", err.Error())
		return
	}

	utils.Success(c, site.ToResponse())
}

// UpdateSite 更新站点
func (h *SiteHandler) UpdateSite(c *gin.Context) {
	id, err := strconv.ParseUint(c.Param("id"), 10, 32)
	if err != nil {
		utils.Error(c, http.StatusBadRequest, "Invalid request", "invalid site id")
		return
	}

	var req models.SiteRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		utils.Error(c, http.StatusBadRequest, "Invalid request", err.Error())
		return
	}

	site, err := h.service.UpdateSite(uint(id), &req)
	if err != nil {
		logger.Error("update site failed", logger.ErrorField(err))
		utils.Error(c, http.StatusInternalServerError, "Failed to update site", err.Error())
		return
	}

	utils.Success(c, site.ToResponse())
}

// DeleteSite 删除站点
func (h *SiteHandler) DeleteSite(c *gin.Context) {
	id, err := strconv.ParseUint(c.Param("id"), 10, 32)
	if err != nil {
		utils.Error(c, http.StatusBadRequest, "Invalid request", "invalid site id")
		return
	}

	if err := h.service.DeleteSite(uint(id)); err != nil {
		logger.Error("delete site failed", logger.ErrorField(err))
		utils.Error(c, http.StatusInternalServerError, "Failed to delete site", err.Error())
		return
	}

	utils.Success(c, gin.H{"message": "site deleted"})
}

// EnableSite 启用站点
func (h *SiteHandler) EnableSite(c *gin.Context) {
	id, err := strconv.ParseUint(c.Param("id"), 10, 32)
	if err != nil {
		utils.Error(c, http.StatusBadRequest, "Invalid request", "invalid site id")
		return
	}

	site, err := h.service.EnableSite(uint(id))
	if err != nil {
		logger.Error("enable site failed", logger.ErrorField(err))
		utils.Error(c, http.StatusInternalServerError, "Failed to enable site", err.Error())
		return
	}

	utils.Success(c, site.ToResponse())
}

// DisableSite 禁用站点
func (h *SiteHandler) DisableSite(c *gin.Context) {
	id, err := strconv.ParseUint(c.Param("id"), 10, 32)
	if err != nil {
		utils.Error(c, http.StatusBadRequest, "Invalid request", "invalid site id")
		return
	}

	site, err := h.service.DisableSite(uint(id))
	if err != nil {
		logger.Error("disable site failed", logger.ErrorField(err))
		utils.Error(c, http.StatusInternalServerError, "Failed to disable site", err.Error())
		return
	}

	utils.Success(c, site.ToResponse())
}

// GetSiteConfig 获取站点配置
func (h *SiteHandler) GetSiteConfig(c *gin.Context) {
	id, err := strconv.ParseUint(c.Param("id"), 10, 32)
	if err != nil {
		utils.Error(c, http.StatusBadRequest, "Invalid request", "invalid site id")
		return
	}

	config, err := h.service.GetSiteConfig(uint(id))
	if err != nil {
		utils.Error(c, http.StatusNotFound, "Config not found", err.Error())
		return
	}

	utils.Success(c, gin.H{"config": config})
}

// GetSiteStats 获取站点统计
func (h *SiteHandler) GetSiteStats(c *gin.Context) {
	stats, err := h.service.GetSiteStats()
	if err != nil {
		logger.Error("get site stats failed", logger.ErrorField(err))
		utils.Error(c, http.StatusInternalServerError, "Failed to get stats", err.Error())
		return
	}

	utils.Success(c, stats)
}

// GenerateAllConfigs 生成所有配置
func (h *SiteHandler) GenerateAllConfigs(c *gin.Context) {
	if err := h.service.GenerateAllConfigs(); err != nil {
		logger.Error("generate all configs failed", logger.ErrorField(err))
		utils.Error(c, http.StatusInternalServerError, "Failed to generate configs", err.Error())
		return
	}

	utils.Success(c, gin.H{"message": "all configs generated"})
}

// ExportSites 导出站点
func (h *SiteHandler) ExportSites(c *gin.Context) {
	data, err := h.service.ExportSites()
	if err != nil {
		logger.Error("export sites failed", logger.ErrorField(err))
		utils.Error(c, http.StatusInternalServerError, "Failed to export sites", err.Error())
		return
	}

	c.Header("Content-Type", "application/json")
	c.Header("Content-Disposition", "attachment; filename=sites.json")
	c.Data(http.StatusOK, "application/json", data)
}

// ImportSites 导入站点
func (h *SiteHandler) ImportSites(c *gin.Context) {
	file, err := c.FormFile("file")
	if err != nil {
		utils.Error(c, http.StatusBadRequest, "Invalid request", "file is required")
		return
	}

	fileContent, err := file.Open()
	if err != nil {
		utils.Error(c, http.StatusInternalServerError, "Failed to read file", err.Error())
		return
	}
	defer fileContent.Close()

	data := make([]byte, file.Size)
	if _, err := fileContent.Read(data); err != nil {
		utils.Error(c, http.StatusInternalServerError, "Failed to read file", err.Error())
		return
	}

	if err := h.service.ImportSites(data); err != nil {
		logger.Error("import sites failed", logger.ErrorField(err))
		utils.Error(c, http.StatusInternalServerError, "Failed to import sites", err.Error())
		return
	}

	utils.Success(c, gin.H{"message": "sites imported"})
}
