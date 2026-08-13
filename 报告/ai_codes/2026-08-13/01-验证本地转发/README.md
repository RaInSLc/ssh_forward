# 验证本地转发辅助程序

## 用途

读取项目根目录 `config.json` 中已存在的 DPAPI 密文，使用当前 Windows 用户解密后临时启动 OpenSSH 本地转发，并向指定本地端口发送 HTTP 请求，输出脱敏的连通性结果和 OpenSSH 错误摘要。

## 运行方式

在该目录执行：

```powershell
$env:CARGO_HOME = "..\..\..\.cargo"
cargo run --manifest-path Cargo.toml
```

## 输入

- 项目根目录的 `config.json`。
- 当前 Windows 用户可解密的 DPAPI 密文。

## 输出

- 本地转发是否开始监听。
- HTTP 响应状态行或脱敏的 OpenSSH 错误摘要。

不会输出密码、DPAPI 密文或完整 SSH 命令。

## 依赖

- Rust/Cargo。
- Windows OpenSSH。
- 项目本地 `ssh-forward-config` crate。

## 可删除性

该目录是本次测试的 AI 辅助代码，不被项目构建、发布或运行流程加载；完成验证后可以安全删除。
