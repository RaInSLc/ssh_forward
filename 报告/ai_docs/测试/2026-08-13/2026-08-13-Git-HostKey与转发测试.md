# Git、Host Key 与转发测试

- 任务名称：Git、Host Key 与转发测试
- 日期：2026-08-13
- 状态：通过
- 负责人：AI
- 关联文件：`apps/desktop/`、`crates/`、`报告/ai_codes/2026-08-13/01-验证本地转发/`

## 已执行验证

| 验证 | 结果 |
| --- | --- |
| 测试主机 SSH 22 TCP 连通性 | 通过 |
| 受控 SSH Local Tunnel `127.0.0.1:12395` 至远端 80 | 通过 |
| 本地 HTTP 请求 | 返回 `HTTP/1.1 200 OK` |
| `cargo test --workspace` | 通过，6 个单元测试通过 |
| `cargo clippy --workspace -- -D warnings` | 通过 |
| `npm run check` | 通过 |
| `npm run build` | 通过 |
| `tauri build --no-bundle` | 通过 |
| 新 Release 启动存活检查 | 通过 |

## 说明

- 真实转发验证只输出 HTTP 状态行，未输出或归档密码。
- 初次测试因 Host Key 未被验证而失败；用户随后明确确认该主机指纹，并完成正常 SSH 登录，后续测试通过。
