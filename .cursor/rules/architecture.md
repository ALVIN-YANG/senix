# Senix Architecture & Git Workflows

1. **Project Principles**:
   - Senix is a zero-overhead Nginx Control Plane API Gateway.
   - Core philosophy: Decouple Data Plane (Nginx) from Control Plane (Go).
   - We support three Work Modes: Standalone, CertOnly, ConfigOnly.

2. **Git Commit Standard**:
   - Use Conventional Commits (`feat:`, `fix:`, `docs:`, `style:`, `refactor:`, `test:`, `chore:`).
   - Keep commits granular and descriptive.

3. **Install/Uninstall Lifecycle**:
   - Update `install.sh` and `uninstall.sh` if any new system-level dependency (e.g., SQLite, ModSecurity) is added to the architecture.

4. **Let's Encrypt**:
   - Integration uses the `lego` library. Maintain automatic cron renewal in the background (`internal/server/server.go`).