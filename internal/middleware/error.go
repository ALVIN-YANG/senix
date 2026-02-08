package middleware

import (
	"net/http"

	"github.com/gin-gonic/gin"
	"go.uber.org/zap"

	"senix/internal/logger"
	"senix/pkg/utils"
)

// ErrorHandler 全局错误处理中间件
func ErrorHandler() gin.HandlerFunc {
	return func(c *gin.Context) {
		c.Next()

		// 检查是否有错误
		if len(c.Errors) > 0 {
			err := c.Errors.Last()
			logger.Error("request error",
				zap.String("path", c.Request.URL.Path),
				zap.String("method", c.Request.Method),
				zap.Error(err),
			)

			// 返回统一错误响应
			c.JSON(http.StatusInternalServerError, utils.ErrorResponse(
				http.StatusInternalServerError,
				"Internal Server Error",
				err.Error(),
			))
		}
	}
}

// Recovery 自定义恢复中间件
func Recovery() gin.HandlerFunc {
	return gin.CustomRecovery(func(c *gin.Context, recovered interface{}) {
		logger.Error("panic recovered",
			zap.String("path", c.Request.URL.Path),
			zap.Any("error", recovered),
		)

		c.JSON(http.StatusInternalServerError, utils.ErrorResponse(
			http.StatusInternalServerError,
			"Internal Server Error",
			"Something went wrong",
		))
	})
}
