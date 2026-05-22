# 当前状态复核

## 复核口径

本复核基于当前 `master` 工作树，只覆盖真机烟测前可以在本地确认的工程状态、文档状态和后续工作边界。Linux x86_64 与 Linux aarch64/ARM64 真机烟测暂不执行，必须等部署准备工作完成后再按 [稳定版真机烟测清单](smoke-test.md) 验证。

## 已收敛事项

- `wincast-client` 已具备 lib target，`src/main.rs` 只负责调用 `wincast_client::run_default_client()` 并处理退出码。
- 客户端 H.264 渲染循环已使用短读取超时，下一帧暂时不可用时仍会继续轮询 SDL 输入并发送心跳。
- Linux 客户端默认启动路径已改为 SDL 全屏窗口，启动后先显示加载进度，收到首帧后复用同一窗口渲染远程画面。
- 客户端已将宿主端 `ProgramExited` 会话结束识别为正常退出，不再把目标程序主动退出显示为运行失败。
- Host session 事件层已使用 `DesktopSessionError` 与 `SessionEventError` 分类错误，不再把 public API 固定为 `String` 错误。
- README 的宿主端配置示例已包含 `[video]` 与 `[capture]` 必填段。
- `wincast-media` 的 fake H.264 测试后端已通过 `test-support` feature 隔离，默认不会进入普通 public API。
- 稳定版捕获边界已收敛为单显示器整屏捕获，不再维护窗口捕获、捕获模式选择和窗口标题定位。
- Host 配置已改为 `[program]`、`[video]`、`[capture]` 三段，`program.startup_delay_ms` 表示启动程序后的可取消等待时间。
- `capture.first_frame_timeout_ms` 只表示整屏捕获启动后的首帧保护超时。
- Windows host 捕获实现固定为 DXGI Desktop Duplication，面向 Windows 10 1809 / Build 17763。
- Host 在启动延迟期间可以响应客户端停止、连接断开、桌面会话不可用和目标程序退出，不再必须等延迟结束后才清理。
- Host 会话期间会监控本次启动的目标程序；目标程序退出时向客户端发送 `ProgramExited` 错误并结束会话。
- Windows 侧启动配置程序时使用 Job Object 管理本次启动的进程树，会话清理时不再只覆盖直接子进程。
- DXGI 输出选择已从固定 adapter 0/output 0 改为枚举 attached desktop 输出；当前稳定版检测到多个可用桌面输出时明确拒绝。
- 协议公开面已删除旧的 `VideoReady` 和 `WindowNotFound`。

## 当前仍未闭合

- 未执行 Windows host 到 Linux x86_64 client 的真机端到端烟测。
- Linux x86_64 与 Linux aarch64/ARM64 真机上的 SDL2 全屏窗口、加载页、OpenH264、渲染和输入回传仍未验证。
- 锁屏、解锁后的完整自动恢复编排仍未完成，当前只能按文档口径处理为感知、拒绝、断开或后续重连尝试。
- 打包和部署步骤需要先形成稳定产物、配置路径、防火墙和目标机依赖检查，再进入真机烟测。

## 本地验证状态

- `cargo fmt --all` 已在当前工作树执行通过。
- `cargo test --workspace --all-targets --all-features` 已在当前工作树执行通过。
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` 已在当前工作树执行通过。

## 下一步工作顺序

- 按部署准备文档准备 Windows host 与 Linux client 的实际产物、配置、依赖和防火墙前置项。
- 等部署准备完成并具备目标机器后，按 `docs/smoke-test.md` 执行 Windows host、Linux x86_64 client 和 Linux ARM64 client 真机烟测。
