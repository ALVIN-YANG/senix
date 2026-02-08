package models

import (
	"time"

	"gorm.io/gorm"
)

// BaseModel 基础模型
type BaseModel struct {
	ID        uint           `json:"id" gorm:"primaryKey"`
	CreatedAt time.Time      `json:"created_at"`
	UpdatedAt time.Time      `json:"updated_at"`
	DeletedAt gorm.DeletedAt `json:"deleted_at,omitempty" gorm:"index"`
}

// User 用户模型
type User struct {
	BaseModel
	Username string `json:"username" gorm:"size:100;uniqueIndex;not null"`
	Password string `json:"-" gorm:"size:255;not null"`
	Email    string `json:"email" gorm:"size:255"`
	Role     string `json:"role" gorm:"size:20;default:'admin'"`
	Active   bool   `json:"active" gorm:"default:true"`
}

// TableName 指定表名
func (User) TableName() string {
	return "users"
}
