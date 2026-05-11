# 开发说明

## Rust 版本

工程使用 Rust 2024 edition，最低 Rust 版本为 1.88。

## 常用验证

```powershell
cargo fmt --all
cargo test --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## 客户端 ARM64 目标

客户端需要覆盖 Linux x86_64 与 Linux aarch64/ARM64：

```powershell
rustup target add aarch64-unknown-linux-gnu
cargo check -p wincast-client --target aarch64-unknown-linux-gnu
```

当前客户端使用 SDL2 创建 Linux 窗口并显示 raw BGRA 帧，这是当前默认可用的画面链路。协议与配置层虽然已经预留 `video.codec = "h264"`、`EncodedVideoFrame` 等扩展点，但这部分目前仅用于协议边界和后续扩展预留，运行期还没有接入 H.264 编码传输或 WebRTC。银河麒麟 V10 等目标机需要安装 SDL2 开发包后再执行 Linux 目标构建；H.264/WebRTC 仍应按后续性能优化方向理解，当前 Windows 开发机上的 workspace 验证只能证明非 Linux 占位路径和协议逻辑可编译，不能替代目标系统真机构建。

## 运行与占位边界

客户端 `run` 支持启动连接阶段的有限重试，便于宿主端前台进程刚启动或端口短暂不可用时验证连接恢复：

```powershell
cargo run -p wincast-client -- --config wincast-client.toml run --retries 3 --retry-delay-ms 1000
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

Service 侧还提供最小 Agent 状态查询 coordinator，可验证 `QueryStatus` / `StatusChanged` 的编排语义：

```powershell
cargo test -p wincast-host service_agent
```

这仍不代表已经接入命名管道权限模型、Service 拉起 Agent、重连、心跳超时、完整会话命令编排或消息投递重试策略。

`wincast-host service` 子命令当前通过可测试的 `ServiceManager` 占位抽象固定安装、卸载、启动、停止和状态查询的 CLI 边界。它仍不会执行真实 Windows Service 操作；验证时应把结果理解为管理抽象占位行为，而不是系统服务已安装或已运行。
