package handler

import (
	"net/http"

	"github.com/gin-gonic/gin"

	"senix/internal/config"
	"senix/internal/database"
	"senix/internal/middleware"
	"senix/internal/models"
	"senix/pkg/utils"
)

type LoginRequest struct {
	Username string `json:"username" binding:"required"`
	Password string `json:"password" binding:"required"`
}

type LoginResponse struct {
	Token string `json:"token"`
	User  struct {
		Username string `json:"username"`
		Role     string `json:"role"`
	} `json:"user"`
}

// HandleLogin 处理登录请求
func HandleLogin(cfg *config.Config) gin.HandlerFunc {
	return func(c *gin.Context) {
		var req LoginRequest
		if err := c.ShouldBindJSON(&req); err != nil {
			utils.Error(c, http.StatusBadRequest, "Invalid request", err.Error())
			return
		}

		var user models.User
		if err := database.DB.Where("username = ?", req.Username).First(&user).Error; err != nil {
			utils.Error(c, http.StatusUnauthorized, "Login failed", "Invalid username or password")
			return
		}

		if !user.CheckPassword(req.Password) {
			utils.Error(c, http.StatusUnauthorized, "Login failed", "Invalid username or password")
			return
		}

		if !user.Active {
			utils.Error(c, http.StatusForbidden, "Login failed", "User account is disabled")
			return
		}

		token, err := middleware.GenerateToken(user.ID, user.Username, user.Role, cfg.Server.JWTSecret)
		if err != nil {
			utils.Error(c, http.StatusInternalServerError, "Login failed", "Failed to generate token")
			return
		}

		resp := LoginResponse{
			Token: token,
		}
		resp.User.Username = user.Username
		resp.User.Role = user.Role

		utils.Success(c, resp)
	}
}

// HandleLogout 处理登出请求
func HandleLogout() gin.HandlerFunc {
	return func(c *gin.Context) {
		// 客户端清除 token 即可
		utils.Success(c, nil)
	}
}
