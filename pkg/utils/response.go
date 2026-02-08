package utils

import (
	"github.com/gin-gonic/gin"
)

// Response 统一响应结构
type Response struct {
	Code    int         `json:"code"`
	Message string      `json:"message"`
	Data    interface{} `json:"data,omitempty"`
}

// SuccessResponse 成功响应
func SuccessResponse(data interface{}) Response {
	return Response{
		Code:    200,
		Message: "success",
		Data:    data,
	}
}

// ErrorResponse 错误响应
func ErrorResponse(code int, message, detail string) Response {
	return Response{
		Code:    code,
		Message: message,
		Data: map[string]string{
			"detail": detail,
		},
	}
}

// PageData 分页数据
type PageData struct {
	List     interface{} `json:"list"`
	Total    int64       `json:"total"`
	Page     int         `json:"page"`
	PageSize int         `json:"page_size"`
}

// SuccessPage 成功分页响应
func SuccessPage(list interface{}, total int64, page, pageSize int) Response {
	return Response{
		Code:    200,
		Message: "success",
		Data: PageData{
			List:     list,
			Total:    total,
			Page:     page,
			PageSize: pageSize,
		},
	}
}

// JSON 返回 JSON 响应
func JSON(c *gin.Context, code int, response Response) {
	c.JSON(code, response)
}

// Success 返回成功响应
func Success(c *gin.Context, data interface{}) {
	c.JSON(200, SuccessResponse(data))
}

// Error 返回错误响应
func Error(c *gin.Context, code int, message, detail string) {
	c.JSON(code, ErrorResponse(code, message, detail))
}
