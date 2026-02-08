# Senix Makefile

.PHONY: all build run test clean docker

# 变量
BINARY_NAME=senix
VERSION=1.0.0
BUILD_TIME=$(shell date +%Y-%m-%d-%H:%M:%S)
LDFLAGS=-ldflags "-X main.Version=$(VERSION) -X main.BuildTime=$(BUILD_TIME)"

# 默认目标
all: build

# 构建
build:
	go build $(LDFLAGS) -o bin/$(BINARY_NAME) cmd/senix/main.go

# 运行
run:
	go run cmd/senix/main.go

# 运行（指定配置）
run-config:
	go run cmd/senix/main.go -config configs/config.yaml

# 测试
test:
	go test -v ./...

# 清理
clean:
	rm -rf bin/
	rm -rf data/
	rm -rf logs/

# 安装依赖
deps:
	go mod download
	go mod tidy

# 格式化代码
fmt:
	go fmt ./...

# 代码检查
lint:
	golangci-lint run

# 构建 Docker 镜像
docker:
	docker build -t senix:$(VERSION) -f deployments/docker/Dockerfile .

# 运行 Docker
docker-run:
	docker-compose up -d

# 停止 Docker
docker-stop:
	docker-compose down

# 开发模式（热重载）
dev:
	air -c .air.toml

# 生成 Swagger 文档
swagger:
	swag init -g cmd/senix/main.go

# 数据库迁移
migrate:
	go run cmd/migrate/main.go

# 帮助
help:
	@echo "Available targets:"
	@echo "  build       - Build the binary"
	@echo "  run         - Run the application"
	@echo "  test        - Run tests"
	@echo "  clean       - Clean build artifacts"
	@echo "  deps        - Download dependencies"
	@echo "  fmt         - Format code"
	@echo "  lint        - Run linter"
	@echo "  docker      - Build Docker image"
	@echo "  docker-run  - Run with Docker Compose"
	@echo "  help        - Show this help"
