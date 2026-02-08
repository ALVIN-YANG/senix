package logger

import (
	"os"
	"path/filepath"

	"go.uber.org/zap"
	"go.uber.org/zap/zapcore"
	"gopkg.in/natefinch/lumberjack.v2"

	"senix/internal/config"
)

var (
	// Log 全局日志实例
	Log *zap.Logger
	// Sugar 全局 SugaredLogger
	Sugar *zap.SugaredLogger
)

// Init 初始化日志
func Init(cfg *config.LogConfig) error {
	encoder := getEncoder(cfg.Format)
	level := getLogLevel(cfg.Level)

	var cores []zapcore.Core

	// 文件输出
	if cfg.Output == "file" || cfg.Output == "both" {
		writer := getLogWriter(cfg)
		fileCore := zapcore.NewCore(encoder, zapcore.AddSync(writer), level)
		cores = append(cores, fileCore)
	}

	// 控制台输出
	if cfg.Output == "stdout" || cfg.Output == "both" {
		consoleEncoder := getEncoder("console")
		consoleCore := zapcore.NewCore(consoleEncoder, zapcore.AddSync(os.Stdout), level)
		cores = append(cores, consoleCore)
	}

	// 创建 Logger
	core := zapcore.NewTee(cores...)
	Log = zap.New(core, zap.AddCaller(), zap.AddStacktrace(zapcore.ErrorLevel))
	Sugar = Log.Sugar()

	return nil
}

// getEncoder 获取编码器
func getEncoder(format string) zapcore.Encoder {
	encoderConfig := zapcore.EncoderConfig{
		TimeKey:        "time",
		LevelKey:       "level",
		NameKey:        "logger",
		CallerKey:      "caller",
		FunctionKey:    zapcore.OmitKey,
		MessageKey:     "msg",
		StacktraceKey:  "stacktrace",
		LineEnding:     zapcore.DefaultLineEnding,
		EncodeLevel:    zapcore.LowercaseLevelEncoder,
		EncodeTime:     zapcore.ISO8601TimeEncoder,
		EncodeDuration: zapcore.SecondsDurationEncoder,
		EncodeCaller:   zapcore.ShortCallerEncoder,
	}

	if format == "json" {
		return zapcore.NewJSONEncoder(encoderConfig)
	}

	// 控制台格式
	encoderConfig.EncodeLevel = zapcore.CapitalColorLevelEncoder
	return zapcore.NewConsoleEncoder(encoderConfig)
}

// getLogLevel 获取日志级别
func getLogLevel(level string) zapcore.Level {
	switch level {
	case "debug":
		return zapcore.DebugLevel
	case "info":
		return zapcore.InfoLevel
	case "warn":
		return zapcore.WarnLevel
	case "error":
		return zapcore.ErrorLevel
	default:
		return zapcore.InfoLevel
	}
}

// getLogWriter 获取日志写入器
func getLogWriter(cfg *config.LogConfig) *lumberjack.Logger {
	// 确保日志目录存在
	dir := filepath.Dir(cfg.FilePath)
	if err := os.MkdirAll(dir, 0755); err != nil {
		panic(err)
	}

	return &lumberjack.Logger{
		Filename:   cfg.FilePath,
		MaxSize:    cfg.MaxSize,    // MB
		MaxBackups: cfg.MaxBackups,
		MaxAge:     cfg.MaxAge,     // days
		Compress:   true,
	}
}

// Sync 同步日志
func Sync() {
	if Log != nil {
		_ = Log.Sync()
	}
}

// Debug 调试日志
func Debug(msg string, fields ...zap.Field) {
	if Log != nil {
		Log.Debug(msg, fields...)
	}
}

// Info 信息日志
func Info(msg string, fields ...zap.Field) {
	if Log != nil {
		Log.Info(msg, fields...)
	}
}

// Warn 警告日志
func Warn(msg string, fields ...zap.Field) {
	if Log != nil {
		Log.Warn(msg, fields...)
	}
}

// Error 错误日志
func Error(msg string, fields ...zap.Field) {
	if Log != nil {
		Log.Error(msg, fields...)
	}
}

// Fatal 致命日志
func Fatal(msg string, fields ...zap.Field) {
	if Log != nil {
		Log.Fatal(msg, fields...)
	}
}

// String 创建字符串字段
func String(key, val string) zap.Field {
	return zap.String(key, val)
}

// Error 创建错误字段
func ErrorField(err error) zap.Field {
	return zap.Error(err)
}
