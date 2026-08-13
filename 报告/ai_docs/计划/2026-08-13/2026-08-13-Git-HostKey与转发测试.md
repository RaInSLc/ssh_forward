# Git、Host Key 与转发测试

- 任务名称：Git、Host Key 与转发测试
- 日期：2026-08-13
- 状态：进行中
- 负责人：AI
- 关联文件：`.gitignore`、`apps/desktop/`、`config.json`、`报告/ai_codes/2026-08-13/01-验证本地转发/`

## 目标

将项目初始化并推送至用户授权的 GitHub 仓库；实现 GUI 的 SSH Host Key 正常确认流程；验证测试 Tunnel `127.0.0.1:12395` 到远端 `10.0.0.103:80`。

## 已知事实与授权

- 用户明确提供目标 GitHub 仓库地址并要求提交版本管理。
- 用户已通过 OpenSSH 的交互流程确认测试主机 ED25519 指纹；当前本机 `known_hosts` 已包含该主机的 ED25519、RSA 和 ECDSA 记录。
- 本机到测试主机 22 端口连通，本地 12395 端口空闲。

## 安全与范围

- Git 不提交 `config.json`、密码密文、`known_hosts`、构建产物、依赖缓存、Node modules 或 AI 辅助测试产物。
- GUI 首次未知 Host Key 显示指纹与“仅本次信任 / 信任并保存 / 取消”选择；不再把普通首次连接仅显示为笼统 OpenSSH 退出错误。
- 本次实际转发测试使用用户提供的测试账户；密码不会写入 Git、配置明文、命令输出或过程文档。

## 验证计划

1. 创建 `.gitignore`，初始化 Git 并检查准备提交的文件不含敏感数据。
2. 为 GUI 增加 Host Key 检测和确认命令及对话框。
3. 使用已确认 Host Key 验证 SSH Local Tunnel 与本地 HTTP 请求。
4. 执行格式、测试、Clippy、前端构建和 Release Build，审查后提交并推送。
