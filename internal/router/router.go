package router

import (
	"net/http"

	"github.com/gin-gonic/gin"

	"senix/internal/config"
	"senix/internal/handler"
	"senix/internal/middleware"
	"senix/pkg/utils"
)

var (
	certHandler *handler.CertHandler
	siteHandler *handler.SiteHandler
)

// Setup 设置路由
func Setup(cfg *config.Config) *gin.Engine {
	// 设置 Gin 模式
	gin.SetMode(cfg.Server.Mode)

	// 创建路由
	r := gin.New()

	// 使用中间件
	r.Use(gin.Logger())
	r.Use(middleware.Recovery())
	r.Use(middleware.ErrorHandler())

	// 初始化处理器
	certHandler = handler.NewCertHandler(cfg)
	if err := certHandler.Initialize(); err != nil {
		panic(err)
	}

	siteHandler = handler.NewSiteHandler(cfg)
	if err := siteHandler.Initialize(); err != nil {
		panic(err)
	}

	// 健康检查（不需要认证）
	r.GET("/api/health", HealthCheck)

	// API 路由组
	api := r.Group("/api")
	{
		// 认证相关（不需要 JWT）
		auth := api.Group("/auth")
		{
			auth.POST("/login", handler.HandleLogin(cfg))
			auth.POST("/logout", handler.HandleLogout())
		}

		// 需要认证的路由
		authorized := api.Group("")
		authorized.Use(middleware.JWTAuth(cfg.Server.JWTSecret))
		{
			// 用户管理
			users := authorized.Group("/users")
			{
				users.GET("", handleGetUsers)
				users.GET("/:id", handleGetUser)
				users.POST("", handleCreateUser)
				users.PUT("/:id", handleUpdateUser)
				users.DELETE("/:id", handleDeleteUser)
			}

			// 站点管理
			sites := authorized.Group("/sites")
			{
				sites.GET("", siteHandler.GetSites)
				sites.GET("/stats", siteHandler.GetSiteStats)
				sites.POST("/generate-all", siteHandler.GenerateAllConfigs)
				sites.GET("/export", siteHandler.ExportSites)
				sites.POST("/import", siteHandler.ImportSites)
				sites.GET("/:id", siteHandler.GetSite)
				sites.POST("", siteHandler.CreateSite)
				sites.PUT("/:id", siteHandler.UpdateSite)
				sites.DELETE("/:id", siteHandler.DeleteSite)
				sites.POST("/:id/enable", siteHandler.EnableSite)
				sites.POST("/:id/disable", siteHandler.DisableSite)
				sites.GET("/:id/config", siteHandler.GetSiteConfig)
			}

			// 证书管理
			certs := authorized.Group("/certs")
			{
				certs.GET("", certHandler.GetCertificates)
				certs.GET("/stats", certHandler.GetCertificateStats)
				certs.GET("/dns-providers", certHandler.GetDNSProviders)
				certs.GET("/:id", certHandler.GetCertificate)
				certs.POST("", certHandler.CreateACME)
				certs.POST("/upload", certHandler.UploadCertificate)
				certs.DELETE("/:id", certHandler.DeleteCertificate)
				certs.POST("/:id/renew", certHandler.RenewCertificate)
				certs.GET("/:id/download", certHandler.DownloadCertificate)
			}

			// 安全策略
			security := authorized.Group("/security")
			{
				// IP 黑名单
				security.GET("/ip-blacklist", handleGetIPBlacklist)
				security.POST("/ip-blacklist", handleAddIPToBlacklist)
				security.DELETE("/ip-blacklist/:ip", handleRemoveIPFromBlacklist)

				// 限流策略
				security.GET("/rate-limits", handleGetRateLimits)
				security.POST("/rate-limits", handleCreateRateLimit)
				security.PUT("/rate-limits/:id", handleUpdateRateLimit)
				security.DELETE("/rate-limits/:id", handleDeleteRateLimit)

				// WAF 规则
				security.GET("/waf-rules", handleGetWAFRules)
				security.POST("/waf-rules", handleCreateWAFRule)
				security.PUT("/waf-rules/:id", handleUpdateWAFRule)
				security.DELETE("/waf-rules/:id", handleDeleteWAFRule)
			}

			// 系统管理
			system := authorized.Group("/system")
			{
				system.GET("/info", handleGetSystemInfo)
				system.GET("/logs", handleGetLogs)
				system.POST("/reload", handleReloadSystem)
			}
		}
	}

	return r
}

// HealthCheck 健康检查
func HealthCheck(c *gin.Context) {
	utils.Success(c, gin.H{
		"status":    "healthy",
		"version":   "1.0.0",
		"timestamp": "2024-01-15",
	})
}

func handleGetUsers(c *gin.Context) {
	utils.Error(c, http.StatusNotImplemented, "Not Implemented", "This feature is coming soon")
}

func handleGetUser(c *gin.Context) {
	utils.Error(c, http.StatusNotImplemented, "Not Implemented", "This feature is coming soon")
}

func handleCreateUser(c *gin.Context) {
	utils.Error(c, http.StatusNotImplemented, "Not Implemented", "This feature is coming soon")
}

func handleUpdateUser(c *gin.Context) {
	utils.Error(c, http.StatusNotImplemented, "Not Implemented", "This feature is coming soon")
}

func handleDeleteUser(c *gin.Context) {
	utils.Error(c, http.StatusNotImplemented, "Not Implemented", "This feature is coming soon")
}

func handleGetIPBlacklist(c *gin.Context) {
	utils.Error(c, http.StatusNotImplemented, "Not Implemented", "This feature is coming soon")
}

func handleAddIPToBlacklist(c *gin.Context) {
	utils.Error(c, http.StatusNotImplemented, "Not Implemented", "This feature is coming soon")
}

func handleRemoveIPFromBlacklist(c *gin.Context) {
	utils.Error(c, http.StatusNotImplemented, "Not Implemented", "This feature is coming soon")
}

func handleGetRateLimits(c *gin.Context) {
	utils.Error(c, http.StatusNotImplemented, "Not Implemented", "This feature is coming soon")
}

func handleCreateRateLimit(c *gin.Context) {
	utils.Error(c, http.StatusNotImplemented, "Not Implemented", "This feature is coming soon")
}

func handleUpdateRateLimit(c *gin.Context) {
	utils.Error(c, http.StatusNotImplemented, "Not Implemented", "This feature is coming soon")
}

func handleDeleteRateLimit(c *gin.Context) {
	utils.Error(c, http.StatusNotImplemented, "Not Implemented", "This feature is coming soon")
}

func handleGetWAFRules(c *gin.Context) {
	utils.Error(c, http.StatusNotImplemented, "Not Implemented", "This feature is coming soon")
}

func handleCreateWAFRule(c *gin.Context) {
	utils.Error(c, http.StatusNotImplemented, "Not Implemented", "This feature is coming soon")
}

func handleUpdateWAFRule(c *gin.Context) {
	utils.Error(c, http.StatusNotImplemented, "Not Implemented", "This feature is coming soon")
}

func handleDeleteWAFRule(c *gin.Context) {
	utils.Error(c, http.StatusNotImplemented, "Not Implemented", "This feature is coming soon")
}

func handleGetSystemInfo(c *gin.Context) {
	utils.Error(c, http.StatusNotImplemented, "Not Implemented", "This feature is coming soon")
}

func handleGetLogs(c *gin.Context) {
	utils.Error(c, http.StatusNotImplemented, "Not Implemented", "This feature is coming soon")
}

func handleReloadSystem(c *gin.Context) {
	utils.Error(c, http.StatusNotImplemented, "Not Implemented", "This feature is coming soon")
}

// CheckAndRenewCerts 检查并续期证书
func CheckAndRenewCerts() error {
	if certHandler != nil {
		return certHandler.CheckAndRenewCertificates()
	}
	return nil
}
