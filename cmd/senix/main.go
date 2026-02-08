package main

import (
	"flag"
	"fmt"
	"os"

	"senix/internal/config"
	"senix/internal/logger"
	"senix/internal/server"
)

var (
	configFile = flag.String("config", "", "配置文件路径")
	version    = flag.Bool("version", false, "显示版本信息")
)

const (
	Version   = "1.0.0"
	BuildTime = "2024-01-15"
)

func main() {
	flag.Parse()

	// 显示版本
	if *version {
		fmt.Printf("Senix Gateway v%s (built at %s)\n", Version, BuildTime)
		os.Exit(0)
	}

	// 加载配置
	cfg, err := config.Load(*configFile)
	if err != nil {
		fmt.Fprintf(os.Stderr, "加载配置失败: %v\n", err)
		os.Exit(1)
	}

	// 创建服务器
	srv := server.New(cfg)

	// 初始化
	if err := srv.Init(); err != nil {
		fmt.Fprintf(os.Stderr, "初始化服务器失败: %v\n", err)
		os.Exit(1)
	}

	// 启动服务器
	if err := srv.Start(); err != nil {
		logger.Error("服务器运行错误", logger.ErrorField(err))
		os.Exit(1)
	}

	// 清理资源
	srv.Cleanup()
}
