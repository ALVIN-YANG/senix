package server

import (
	"context"
	"fmt"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/gin-gonic/gin"
	"go.uber.org/zap"

	"senix/internal/config"
	"senix/internal/database"
	"senix/internal/logger"
	"senix/internal/router"
)

// Server HTTP 服务器
type Server struct {
	config *config.Config
	router *gin.Engine
	http   *http.Server
}

// New 创建服务器
func New(cfg *config.Config) *Server {
	return &Server{
		config: cfg,
	}
}

// Init 初始化服务器
func (s *Server) Init() error {
	// 初始化日志
	if err := logger.Init(&s.config.Log); err != nil {
		return fmt.Errorf("init logger failed: %w", err)
	}
	logger.Info("logger initialized")

	// 初始化数据库
	if err := database.Init(&s.config.Database); err != nil {
		return fmt.Errorf("init database failed: %w", err)
	}
	logger.Info("database initialized")

	// 设置路由
	s.router = router.Setup(s.config)

	// 创建 HTTP 服务器
	s.http = &http.Server{
		Addr:    fmt.Sprintf("%s:%d", s.config.Server.Host, s.config.Server.Port),
		Handler: s.router,
	}

	return nil
}

// Start 启动服务器
func (s *Server) Start() error {
	logger.Info("starting server",
		zap.String("addr", s.http.Addr),
		zap.String("mode", s.config.Server.Mode),
	)

	// 启动 HTTP 服务器（非阻塞）
	go func() {
		if err := s.http.ListenAndServe(); err != nil && err != http.ErrServerClosed {
			logger.Fatal("server start failed", zap.Error(err))
		}
	}()

	// 启动证书自动续期任务
	s.startCertRenewalTask()

	// 等待中断信号
	quit := make(chan os.Signal, 1)
	signal.Notify(quit, syscall.SIGINT, syscall.SIGTERM)
	<-quit

	logger.Info("shutting down server...")

	// 优雅关闭
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	if err := s.http.Shutdown(ctx); err != nil {
		logger.Error("server forced to shutdown", zap.Error(err))
		return err
	}

	logger.Info("server exited")
	return nil
}

// Stop 停止服务器
func (s *Server) Stop() error {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	return s.http.Shutdown(ctx)
}

// Cleanup 清理资源
func (s *Server) Cleanup() {
	// 关闭数据库
	if err := database.Close(); err != nil {
		logger.Error("close database failed", zap.Error(err))
	}

	// 同步日志
	logger.Sync()
}

// startCertRenewalTask 启动证书自动续期定时任务
func (s *Server) startCertRenewalTask() {
	// 每天执行一次
	ticker := time.NewTicker(24 * time.Hour)
	go func() {
		for range ticker.C {
			logger.Info("Starting certificate renewal check task...")
			if err := router.CheckAndRenewCerts(); err != nil {
				logger.Error("Certificate renewal check failed", zap.Error(err))
			}
		}
	}()
}

