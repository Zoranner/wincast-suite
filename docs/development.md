# 开发说明

## Rust 版本

工程使用 Rust 2024 edition，最低 Rust 版本为 1.88。

## 常用验证

```powershell
cargo fmt --all
cargo test --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## 客户端目标平台

客户端需要覆盖 Linux x86_64 与 Linux aarch64/ARM64：

```powershell
rustup target add aarch64-unknown-linux-gnu
cargo check -p wincast-client --target aarch64-unknown-linux-gnu
```

当前客户端使用 SDL2 创建 Linux 窗口并显示 raw BGRA 帧，这是当前默认可用的画面链路。协议与配置层虽然已经预留 `video.codec = "h264"`、`EncodedVideoFrame` 等扩展点，但这部分目前仅用于协议边界和后续扩展预留，运行期还没有接入 H.264 编码传输或 WebRTC。

Windows 开发机上的 workspace 验证只能证明非 Linux 占位路径和协议逻辑可编译，不能替代目标系统真机构建。客户端稳定版收口时，必须在目标 Linux 机器上安装 SDL2 开发包后分别验证：

```bash
sudo apt-get update
sudo apt-get install -y pkg-config libsdl2-dev
cargo test -p wincast-protocol -p wincast-client
cargo clippy -p wincast-protocol -p wincast-client --all-targets --all-features -- -D warnings
```

在银河麒麟 V10 等不使用 `apt` 的系统上，应改用系统对应包管理器安装 `pkg-config` 和 SDL2 开发包，再执行同一组 Cargo 命令。x86_64 目标机和 aarch64/ARM64 目标机都需要完成这组验证；CI 中的 aarch64 `cargo check --target aarch64-unknown-linux-gnu` 只能确认 Rust 交叉编译边界，不能替代 ARM64 目标机上的 SDL2 链接和窗口运行验证。

## 运行与占位边界

稳定版真机烟测流程见 [稳定版真机烟测清单](smoke-test.md)。

Host 与 Client 默认从用户配置目录读取配置，日常运行不需要每次传 `--config`。Windows host 默认读取 `%APPDATA%\WinCast\wincast-host.toml`；Linux client 默认读取 `${XDG_CONFIG_HOME:-$HOME/.config}/wincast/wincast-client.toml`。`XDG_CONFIG_HOME` 必须是非空绝对路径；未设置、为空或为相对路径时回退到 `$HOME/.config`。`--config` 仅用于临时调试或一次性验证时覆盖默认路径。

仓库内 `examples/` 目录提供稳定版烟测示例配置。调整示例后至少执行以下校验，确保示例仍可被配置模型解析：

```powershell
cargo test -p wincast-protocol --test config parses_stable
cargo run -p wincast-host -- --config examples/wincast-host.toml validate
cargo run -p wincast-client -- --config examples/wincast-client.toml validate
```

客户端 `run` 支持启动连接阶段的有限重试，便于宿主端前台进程刚启动或端口短暂不可用时验证连接恢复：

```powershell
cargo run -p wincast-client -- run --retries 3 --retry-delay-ms 1000
```

该重试只覆盖初始连接，不等同于会话中断后的自动恢复，也不代表 Service/Agent 编排已经完成。

Service 与 Host Agent IPC 当前已具备长度前缀 JSON frame 编解码底座，可用协议包测试验证：

```powershell
cargo test -p wincast-protocol ipc
```

Host 侧还提供最小 TCP loopback transport，可把现有 `ServiceIpcEndpoint` 接到真实 `TcpStream` 并验证 Service/Agent 双向 round-trip：

```powershell
cargo test -p wincast-host service_ipc::tests::loopback_transport_round_trips_service_and_agent_messages
```

Service 侧还提供最小 Agent request/ack coordinator，可验证 `QueryStatus` / `StatusChanged` 以及 `StartSession` / `StopSession` 的编排语义：

```powershell
cargo test -p wincast-host service_agent
```

这仍不代表已经接入命名管道权限模型、Service 拉起 Agent、重连、心跳超时、真实 Agent 进程会话编排或消息投递重试策略。

`wincast-host service` 子命令已接入 Windows SCM 的安装、卸载、启动、停止和状态查询最小闭环，并提供隐藏的 `service run` 入口供 SCM 启动服务进程。该能力只覆盖系统服务管理本身，不代表 Service 已经拉起交互桌面 Host Agent，也不代表命名管道权限模型、锁屏恢复、心跳或自动重连已经完成。真实 `service install/start/stop/uninstall` 会修改系统服务状态，需要在 Windows 管理员终端中手动烟测，不应放入默认 CI。
