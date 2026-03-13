package database

import (
	"fmt"
	"os"
	"path/filepath"

	"gorm.io/driver/sqlite"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"

	"senix/internal/config"
	"senix/internal/models"
)

// DB 全局数据库实例
var DB *gorm.DB

// Init 初始化数据库
func Init(cfg *config.DatabaseConfig) error {
	// 确保数据库目录存在
	dir := filepath.Dir(cfg.Path)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return fmt.Errorf("create database dir failed: %w", err)
	}

	var err error
	switch cfg.Type {
	case "sqlite":
		DB, err = initSQLite(cfg)
	default:
		return fmt.Errorf("unsupported database type: %s", cfg.Type)
	}

	if err != nil {
		return err
	}

	// 自动迁移
	if err := autoMigrate(); err != nil {
		return fmt.Errorf("auto migrate failed: %w", err)
	}

	// 初始化默认数据
	if err := initDefaultData(); err != nil {
		return fmt.Errorf("init default data failed: %w", err)
	}

	return nil
}

// initDefaultData 初始化默认数据
func initDefaultData() error {
	var count int64
	if err := DB.Model(&models.User{}).Count(&count).Error; err != nil {
		return err
	}

	if count == 0 {
		admin := models.User{
			Username: "admin",
			Role:     "admin",
			Active:   true,
		}
		if err := admin.HashPassword("admin123"); err != nil {
			return err
		}
		if err := DB.Create(&admin).Error; err != nil {
			return err
		}
	}

	var siteCount int64
	if err := DB.Model(&models.Site{}).Count(&siteCount).Error; err != nil {
		return err
	}

	if siteCount == 0 {
		selfSite := models.Site{
			Name:              "Senix Gateway 控制台",
			Domain:            "senix.ilovestudy.club",
			WorkMode:          models.WorkModeConfigOnly,
			Port:              80,
			SSLEnabled:        true,
			Upstream:          "http://127.0.0.1:8080",
			Status:            "active",
			Description:       "Senix 控制台自身配置，请勿随意删除。此配置由 Certbot 提供 HTTPS 保护。",
			ConfigExportPath:  "/etc/nginx/conf.d/senix.conf",
		}
		if err := DB.Create(&selfSite).Error; err != nil {
			return err
		}
	}

	return nil
}

// initSQLite 初始化 SQLite
func initSQLite(cfg *config.DatabaseConfig) (*gorm.DB, error) {
	db, err := gorm.Open(sqlite.Open(cfg.Path), &gorm.Config{
		Logger: logger.Default.LogMode(logger.Silent),
	})
	if err != nil {
		return nil, fmt.Errorf("open sqlite failed: %w", err)
	}

	return db, nil
}

// autoMigrate 自动迁移数据库结构
func autoMigrate() error {
	return DB.AutoMigrate(
		&models.User{},
		&models.Site{},
		&models.Certificate{},
		&models.WAFRule{},
		&models.IPBlacklist{},
		&models.RateLimit{},
	)
}

// Close 关闭数据库连接
func Close() error {
	if DB != nil {
		sqlDB, err := DB.DB()
		if err != nil {
			return err
		}
		return sqlDB.Close()
	}
	return nil
}
