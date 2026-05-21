# 当前状态复核

## 复核口径

本复核基于当前 `master` 工作树，只覆盖真机烟测前可以在本地确认的工程状态、文档状态和后续工作边界。Linux x86_64 与 Linux aarch64/ARM64 真机烟测暂不执行，必须等部署准备工作完成后再按 [稳定版真机烟测清单](smoke-test.md) 验证。

## 已收敛事项

- `wincast-client` 已具备 lib target，`src/main.rs` 只负责调用 `wincast_client::run_default_client()` 并处理退出码。
- 客户端 H.264 渲染循环已使用短读取超时，下一帧暂时不可用时仍会继续轮询 SDL 输入并发送心跳。
- Host session 事件层已使用 `DesktopSessionError` 与 `SessionEventError` 分类错误，不再把 public API 固定为 `String` 错误。
- README 的宿主端配置示例已包含 `[video]` 与 `[capture]` 必填段。
- `wincast-media` 的 fake H.264 测试后端已通过 `test-support` feature 隔离，默认不会进入普通 public API。

## 当前仍未闭合

- 未执行 Windows host 到 Linux x86_64 client 的真机端到端烟测。
- 未执行 Linux aarch64/ARM64 真机上的 OpenH264、SDL2 构建、窗口创建、渲染和输入回传验证。
- 锁屏、解锁后的完整自动恢复编排仍未完成，当前只能按文档口径处理为感知、拒绝、断开或后续重连尝试。
- 打包和部署步骤需要先形成稳定产物、配置路径、防火墙和目标机依赖检查，再进入真机烟测。

## 本地验证状态

- `cargo fmt --all -- --check` 已在当前工作树执行通过。
- `cargo test --workspace --all-targets --all-features` 初次执行时曾因 crates.io/OpenH264 下载链路和超时无法给出结果；延长超时重试后已执行通过。
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` 已在当前工作树执行通过。

## 下一步工作顺序

- 按部署准备文档准备 Windows host 与 Linux client 的实际产物、配置、依赖和防火墙前置项。
- 持续保持本地 `cargo fmt`、`cargo test` 与 `cargo clippy` 门禁，避免部署准备文档和代码状态脱节。
- 等部署准备完成并具备目标机器后，按 `docs/smoke-test.md` 执行 Windows host、Linux x86_64 client 和 Linux ARM64 client 真机烟测。
