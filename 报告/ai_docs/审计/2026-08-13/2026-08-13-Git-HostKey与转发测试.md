# Git、Host Key 与转发测试审计

- 任务名称：Git、Host Key 与转发测试
- 日期：2026-08-13
- 状态：通过
- 负责人：AI
- 关联文件：`.gitignore`、`.gitattributes`、`apps/desktop/src-tauri/src/main.rs`、`README.md`

## 变更范围

- 初始化 Git 仓库并设置用户提供的 GitHub 远程地址。
- 排除运行时配置、SSH 信任数据、构建与依赖产物、AI 辅助构建产物和本地运行日志。
- 增加首次连接自动接受 Host Key 的实现与文档说明。

## 安全检查

- `config.json` 及其 DPAPI 密文未加入暂存区。
- `known_hosts`、构建输出、Node modules、Cargo 缓存和辅助程序目标目录均由忽略规则排除。
- 自动接受仅发生在当前主机没有记录时；已有或变更的 Host Key 继续由 OpenSSH 严格校验，未使用关闭校验的参数。

## 遗留风险

- 自动首次信任和手动输入 `yes` 一样，首次获取 Host Key 的网络链路若遭受中间人攻击会信任攻击者密钥。需要更高安全保证时，应改为在 GUI 中显示指纹并由用户与可信来源核验后确认。
