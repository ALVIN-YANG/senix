# Senix Go Backend Rules

1. **Architecture & Structure**:
   - Strictly follow the layered architecture: `cmd` -> `internal/server` -> `internal/router` -> `internal/handler` -> `internal/service` (or specific domain like `internal/cert`) -> `internal/database` -> `internal/models`.
   - Never inject HTTP-specific dependencies (`*gin.Context`, `http.Request`) into the Service layer.

2. **Database & ORM (GORM)**:
   - Always update the `internal/models` schemas before executing queries to ensure consistency.
   - Run `autoMigrate` in `internal/database/database.go` for new tables.

3. **Error Handling**:
   - Always wrap errors with context: `fmt.Errorf("do something failed: %w", err)`.
   - Use `zap` structured logging in `logger.Error` to trace variables: `logger.Error("msg", logger.ErrorField(err), zap.String("var", val))`.

4. **Nginx Integration**:
   - The Go control plane only modifies the state and templates. It uses the `internal/nginx` package to execute commands (`nginx -t`, `nginx -s reload`).

5. **Security**:
   - Handlers parsing JSON must use `ShouldBindJSON`.
   - Never expose passwords or raw tokens in JSON responses (`json:"-"` in models).