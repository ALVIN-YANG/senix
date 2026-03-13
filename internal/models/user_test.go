package models

import (
	"testing"
)

func TestUserPasswordHashing(t *testing.T) {
	user := &User{
		Username: "testuser",
		Role:     "admin",
	}

	password := "mysecurepassword"

	err := user.HashPassword(password)
	if err != nil {
		t.Fatalf("Failed to hash password: %v", err)
	}

	if user.Password == "" || user.Password == password {
		t.Errorf("Password was not properly hashed")
	}

	if !user.CheckPassword(password) {
		t.Errorf("CheckPassword failed for correct password")
	}

	if user.CheckPassword("wrongpassword") {
		t.Errorf("CheckPassword succeeded for wrong password")
	}
}