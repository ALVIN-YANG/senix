# Senix @SENIX_VERSION@

Prebuilt HTTP gateway with scoped MCP tools and an embedded Owner console.
预编译 HTTP 网关，提供受限 MCP 工具和内置 Owner 管理后台。

## Prerequisites / 前提

- Use the archive for your OS and architecture. Linux requires glibc 2.34 or newer.
  选择与系统和 CPU 架构匹配的归档；Linux 要求 glibc 2.34 或更新版本。
- Run the commands below in Bash, from the extracted directory. Rust, Node and Docker are not needed to run this binary. curl is used only for the checks below.
  在解压目录中使用 Bash 执行下方命令。运行二进制不需要 Rust、Node 或 Docker；下方检查使用 curl。
- Provide reachable application backends and free ports 8080/9080, or change the example addresses and listen ports.
  先准备可访问的业务后端和空闲的 8080/9080 端口；也可以修改示例地址与监听端口。

## 1. Verify the program / 检查程序

```bash
./senixd --version
```

An `edge-<commit>` archive label identifies the source commit; the program reports the Cargo package version.
`edge-<commit>` 归档标签标识源码提交；程序显示 Cargo 包版本。

## 2. Configure backends / 配置后端

Edit `examples/gateway.json` before starting. The included example routes `example.test` to `127.0.0.1:4101` and `127.0.0.1:4102`. Backend A must answer `GET /health` with HTTP 200; backend B uses a TCP check. The example checks every five seconds and requires two consecutive successes.

先编辑 `examples/gateway.json`。示例把 `example.test` 代理到 `127.0.0.1:4101` 和 `127.0.0.1:4102`。后端 A 的 `GET /health` 必须返回 HTTP 200，后端 B 使用 TCP 检查。示例每五秒检查一次，需要连续两次成功。

## 3. Create the local Owner / 创建本地 Owner

Run once on the gateway machine. The password must have 12–256 characters; Bash reads it without displaying it or putting it in command arguments. Successful Owner creation consumes the one-time bootstrap credential printed by the first command.

在网关本机执行一次。密码要求 12–256 个字符；Bash 隐藏输入，不把密码放入命令参数。Owner 创建成功后，第一条命令打印的一次性引导凭据失效。

```bash
umask 077
./senixd credential bootstrap --db senix.db --label local-owner
read -r -s -p 'Owner password (12-256 characters): ' SENIX_OWNER_PASSWORD
printf '\n'
printf '%s' "$SENIX_OWNER_PASSWORD" | ./senixd owner bootstrap \
  --db senix.db --username admin --password-stdin
unset SENIX_OWNER_PASSWORD
```

If account creation fails, correct the password and repeat the password prompt and Owner command. Keep the same database; do not rerun credential bootstrap.
账号创建失败时，修正密码并重复密码输入和 Owner 命令。保留同一数据库，不要重复创建引导凭据。

## 4. Start the gateway / 启动网关

```bash
./senixd --listen 127.0.0.1:8080 --admin-listen 127.0.0.1:9080 \
  --db senix.db --config examples/gateway.json
```

This runs in the foreground. Keep this terminal open; use another terminal for the checks. Both listeners bind to loopback in this example. Change the proxy listen address if other machines must reach it.
程序在前台运行。保留此终端，在另一终端中检查。示例的代理和管理端口都绑定 loopback；其他机器需要访问时，修改代理监听地址。

## 5. Check and log in / 检查并登录

```bash
curl --fail http://127.0.0.1:9080/healthz
curl --fail -H 'Host: example.test' http://127.0.0.1:8080/
```

Wait for the backends to pass their health checks before checking proxy traffic. Open **http://127.0.0.1:9080/admin/** and log in as `admin` with the password you entered. The language selector switches between English and Simplified Chinese.
等待后端通过健康检查后，再检查代理请求。打开 **http://127.0.0.1:9080/admin/**，使用 `admin` 和刚输入的密码登录。语言选择器可切换英文和简体中文。

## Data and reference / 数据与文档

Keep `senix.db` in a persistent directory. Restart with the same database: saved configuration takes priority over the example file. Use the documented backup command before moving or upgrading a live installation.
将 `senix.db` 保存在持久目录。使用同一数据库重启时，已保存配置优先于示例文件。迁移或升级运行实例前，使用文档中的备份命令。

- [English reference](https://github.com/ALVIN-YANG/senix/blob/@SENIX_DOC_REF@/README.en.md) · [中文说明](https://github.com/ALVIN-YANG/senix/blob/@SENIX_DOC_REF@/README.md)
- [AI configuration workflow](https://github.com/ALVIN-YANG/senix/blob/@SENIX_DOC_REF@/docs/ai-configuration.en.md) · [AI 配置流程](https://github.com/ALVIN-YANG/senix/blob/@SENIX_DOC_REF@/docs/ai-configuration.md)
- [Monitoring / 监控](https://github.com/ALVIN-YANG/senix/blob/@SENIX_DOC_REF@/docs/observability.md) · [Backups / 备份](https://github.com/ALVIN-YANG/senix/blob/@SENIX_DOC_REF@/README.en.md#security-backup-and-limits)
