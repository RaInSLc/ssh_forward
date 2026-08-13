# Git、Host Key 与转发测试

- 任务名称：Git、Host Key 与转发测试
- 日期：2026-08-13
- 状态：进行中
- 负责人：AI
- 关联文件：`.gitignore`、`apps/desktop/`、`config.json`、`报告/ai_codes/2026-08-13/01-验证本地转发/`

## 执行记录

1. 已确认项目尚未初始化 Git 仓库；用户提供 GitHub 远程地址并授权提交版本管理。
2. 已确认测试服务器 SSH 22 端口可达；当前 Windows 用户的 `known_hosts` 已含测试主机 ED25519 公钥，和用户交互确认的主机一致。
3. 已将测试 Tunnel 更新为 `127.0.0.1:12395` 至远端 `10.0.0.103:80`。首次受控验证因未使用当前用户 known_hosts 而被严格 Host Key 检查拦截；不涉及密码错误或端口不可达。
4. 将把首次 Host Key 处理改为 GUI 明确确认流程，并在 Git 初始化前排除敏感配置、依赖及构建产物。
5. 已新增 `.gitignore` 并初始化 Git，远程地址已设为用户提供的 GitHub 仓库。`config.json`、known_hosts、构建产物、依赖目录和 AI 辅助构建产物均被排除。
6. 已实现首次 Host Key 自动确认保存：首次 Tunnel 启动扫描 ED25519 公钥并写入当前 Windows 用户 `known_hosts`；后续严格校验，Host Key 变更不自动覆盖。已在 README 说明该行为。
7. 已通过受控验证建立本地 `127.0.0.1:12395` 到远端 `10.0.0.103:80` 的 SSH 转发，本地 HTTP 请求返回 `HTTP/1.1 200 OK`。密码未写入配置明文、命令输出或归档。
8. 已增加 `.gitattributes` 统一文本文件使用 LF；忽略本地 OpenCode 运行日志。已修正非 22 端口 Host Key 的 known_hosts 查找格式。
9. 已完成首个 Git 提交 `f10f970`（`Initial SSH forward application`），并成功推送至 `origin/main`。
