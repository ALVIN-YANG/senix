package models

import (
	"time"

	"golang.org/x/crypto/bcrypt"
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

// CheckPassword 校验密码
func (u *User) CheckPassword(password string) bool {
	err := bcrypt.CompareHashAndPassword([]byte(u.Password), []byte(password))
	return err == nil
}

// HashPassword 生成密码哈希
func (u *User) HashPassword(password string) error {
	bytes, err := bcrypt.GenerateFromPassword([]byte(password), bcrypt.DefaultCost)
	if err != nil {
		return err
	}
	u.Password = string(bytes)
	return nil
}
