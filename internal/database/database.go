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
