package config

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/spf13/viper"
)

// Config 应用配置
type Config struct {
	Server   ServerConfig   `mapstructure:"server"`
	Database DatabaseConfig `mapstructure:"database"`
	Log      LogConfig      `mapstructure:"log"`
	Cert     CertConfig     `mapstructure:"cert"`
	Nginx    NginxConfig    `mapstructure:"nginx"`
}

// ServerConfig 服务器配置
type ServerConfig struct {
	Host      string `mapstructure:"host"`
	Port      int    `mapstructure:"port"`
	Mode      string `mapstructure:"mode"`
	JWTSecret string `mapstructure:"jwt_secret"`
}

// DatabaseConfig 数据库配置
type DatabaseConfig struct {
	Type string `mapstructure:"type"`
	Path string `mapstructure:"path"`
}

// LogConfig 日志配置
type LogConfig struct {
	Level      string `mapstructure:"level"`
	Format     string `mapstructure:"format"`
	Output     string `mapstructure:"output"`
	FilePath   string `mapstructure:"file_path"`
	MaxSize    int    `mapstructure:"max_size"`
	MaxBackups int    `mapstructure:"max_backups"`
	MaxAge     int    `mapstructure:"max_age"`
}

// CertConfig 证书配置
type CertConfig struct {
	DataDir           string `mapstructure:"data_dir"`
	DefaultCA         string `mapstructure:"default_ca"`
	DefaultEmail      string `mapstructure:"default_email"`
	AutoRenew         bool   `mapstructure:"auto_renew"`
	RenewBeforeDays   int    `mapstructure:"renew_before_days"`
}

// NginxConfig Nginx 配置
type NginxConfig struct {
	ConfigDir      string `mapstructure:"config_dir"`
	CertDir        string `mapstructure:"cert_dir"`
	TemplateDir    string `mapstructure:"template_dir"`
	AutoReload     bool   `mapstructure:"auto_reload"`
	ReloadCmd      string `mapstructure:"reload_cmd"`
	TestCmd        string `mapstructure:"test_cmd"`
}

// Default 返回默认配置
func Default() *Config {
	return &Config{
		Server: ServerConfig{
			Host:      "0.0.0.0",
			Port:      8080,
			Mode:      "debug",
			JWTSecret: "change-me-in-production",
		},
		Database: DatabaseConfig{
			Type: "sqlite",
			Path: "./data/db/senix.db",
		},
		Log: LogConfig{
			Level:      "info",
			Format:     "console",
			Output:     "stdout",
			FilePath:   "./logs/senix.log",
			MaxSize:    100,
			MaxBackups: 10,
			MaxAge:     30,
		},
		Cert: CertConfig{
			DataDir:         "./data",
			DefaultCA:       "letsencrypt",
			DefaultEmail:    "",
			AutoRenew:       true,
			RenewBeforeDays: 30,
		},
		Nginx: NginxConfig{
			ConfigDir:      "./data/configs",
			CertDir:        "./data/certs",
			TemplateDir:    "./configs/templates",
			AutoReload:     false,
			ReloadCmd:      "nginx -s reload",
			TestCmd:        "nginx -t",
		},
	}
}

// Load 加载配置
func Load(path string) (*Config, error) {
	cfg := Default()

	if path != "" {
		viper.SetConfigFile(path)
	} else {
		viper.AddConfigPath(".")
		viper.AddConfigPath("./configs")
		viper.SetConfigName("config")
		viper.SetConfigType("yaml")
	}

	viper.SetEnvPrefix("SENIX")
	viper.AutomaticEnv()

	// 设置默认值
	setDefaults(cfg)

	// 读取配置文件
	if err := viper.ReadInConfig(); err != nil {
		if _, ok := err.(viper.ConfigFileNotFoundError); !ok {
			return nil, fmt.Errorf("read config file failed: %w", err)
		}
		// 配置文件不存在，使用默认值
	}

	if err := viper.Unmarshal(cfg); err != nil {
		return nil, fmt.Errorf("unmarshal config failed: %w", err)
	}

	// 确保数据目录存在
	if err := ensureDirs(cfg); err != nil {
		return nil, fmt.Errorf("ensure dirs failed: %w", err)
	}

	return cfg, nil
}

// setDefaults 设置默认值
func setDefaults(cfg *Config) {
	viper.SetDefault("server.host", cfg.Server.Host)
	viper.SetDefault("server.port", cfg.Server.Port)
	viper.SetDefault("server.mode", cfg.Server.Mode)
	viper.SetDefault("database.type", cfg.Database.Type)
	viper.SetDefault("database.path", cfg.Database.Path)
	viper.SetDefault("log.level", cfg.Log.Level)
	viper.SetDefault("log.format", cfg.Log.Format)
	viper.SetDefault("log.output", cfg.Log.Output)
	viper.SetDefault("cert.data_dir", cfg.Cert.DataDir)
	viper.SetDefault("cert.auto_renew", cfg.Cert.AutoRenew)
	viper.SetDefault("cert.renew_before_days", cfg.Cert.RenewBeforeDays)
	viper.SetDefault("nginx.config_dir", cfg.Nginx.ConfigDir)
	viper.SetDefault("nginx.cert_dir", cfg.Nginx.CertDir)
}

// ensureDirs 确保目录存在
func ensureDirs(cfg *Config) error {
	dirs := []string{
		filepath.Dir(cfg.Database.Path),
		filepath.Dir(cfg.Log.FilePath),
		cfg.Cert.DataDir,
		filepath.Join(cfg.Cert.DataDir, "certs"),
		cfg.Nginx.ConfigDir,
		cfg.Nginx.TemplateDir,
	}

	for _, dir := range dirs {
		if dir == "" || dir == "." {
			continue
		}
		if err := os.MkdirAll(dir, 0755); err != nil {
			return fmt.Errorf("create dir %s failed: %w", dir, err)
		}
	}

	return nil
}
