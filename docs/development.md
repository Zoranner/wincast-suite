# 开发说明

## Rust 版本

工程使用 Rust 2024 edition，最低 Rust 版本为 1.88。

## Windows 绑定策略

Windows 侧当前保留 `windows` 与 `windows-sys` 双线绑定，不承诺已经收敛到单一绑定：

- `wincast-capture` 使用 `windows`，因为 Windows Graphics Capture、WinRT 对象和 D3D11 资源交互需要更完整的类型封装和接口调用支持。
- `wincast-host` 与 `wincast-input` 使用 `windows-sys`，因为当前主要调用 WTS、窗口枚举和 `SendInput` 等 Win32 API，边界更接近 C ABI，轻量绑定更便于显式管理句柄、结构体和错误码。

升级 Windows 绑定依赖时，应分别检查两条线的版本兼容性、feature 范围和生成类型变化。跨 crate 传递 Windows 类型时优先使用项目自有的中性类型、句柄值或明确封装，避免把 `windows` 的 COM/WinRT 类型和 `windows-sys` 的裸 FFI 类型扩散到不属于它们的 crate。确需转换时，应在转换点说明来源、生命周期、所有权和失败语义，并把 unsafe 或句柄释放责任限制在最小模块内。

## 常用验证

修改 Rust 代码后先执行格式化修复：

```powershell
cargo fmt --all
```

评审、提交前或 CI 场景使用只读格式检查：

```powershell
cargo fmt --all -- --check
```

完整验证至少包括：

```powershell
cargo test --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## 客户端目标平台

客户端需要覆盖 Linux x86_64 与 Linux aarch64/ARM64：

```powershell
rustup target add aarch64-unknown-linux-gnu
cargo check -p wincast-client --target aarch64-unknown-linux-gnu
```

当前正式视频链路固定为低延迟 H.264 编码帧，不再把未编码像素帧作为稳定版网络传输主线，也不规划 WebRTC 或 UDP 媒体通道。Linux 客户端仍使用 SDL2 承载窗口和输入事件，稳定版收口需要验证 H.264 解码、渲染和输入回传的端到端链路。

Windows 开发机上的 workspace 验证只能证明非 Linux 占位路径和协议逻辑可编译，不能替代目标系统真机构建。客户端稳定版收口时，必须在目标 Linux 机器上安装 SDL2 开发包后分别验证：

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libsdl2-dev
cargo test -p wincast-protocol -p wincast-client
cargo clippy -p wincast-protocol -p wincast-client --all-targets --all-features -- -D warnings
```

OpenH264 后端会在构建时编译 C/C++ 源码，因此 Linux 目标机除 SDL2 外还需要可用的 C/C++ 编译工具链。若构建报找不到 `g++`、`cc`、`c++` 或 OpenH264 build script 失败，先补齐系统编译工具链，再判断 Rust 代码问题。

在银河麒麟 V10 等不使用 `apt` 的系统上，应改用系统对应包管理器安装 C/C++ 编译工具链、`pkg-config` 和 SDL2 开发包，再执行同一组 Cargo 命令。x86_64 目标机和 aarch64/ARM64 目标机都需要完成这组验证，并按稳定版真机烟测清单执行客户端窗口运行与输入回传验证；aarch64 交叉编译检查只能确认 Rust 编译边界，不能替代 ARM64 目标机上的 SDL2 链接和窗口运行验证。

## 运行与占位边界

稳定版真机烟测前，先按 [部署准备说明](deployment-prep.md) 确认两端产物、配置、依赖和防火墙。准备完成后，再执行 [稳定版真机烟测清单](smoke-test.md)。

Host 与 Client 默认从用户配置目录读取配置。Windows host 默认读取 `%APPDATA%\WinCast\host.toml`；Linux client 默认读取 `${XDG_CONFIG_HOME:-$HOME/.config}/wincast/client.toml`。`XDG_CONFIG_HOME` 必须是非空绝对路径；未设置、为空或为相对路径时回退到 `$HOME/.config`。两端执行可执行程序即启动。

仓库内 `examples/` 目录提供稳定版烟测示例配置。调整示例后至少执行以下校验，确保示例仍可被配置模型解析：

```powershell
cargo test -p wincast-protocol --test config parses_stable
```

客户端内置有限重试，覆盖初始连接失败、宿主端会话门禁的可恢复拒绝，以及 H.264 视频流中断后的重新连接。该机制不等同于锁屏/解锁后的完整自动恢复编排，也不能替代真机长时间网络异常验证。

宿主端和客户端仍使用长度前缀 JSON frame 承载控制消息，可用协议包测试验证：

```powershell
cargo test -p wincast-protocol ipc
```

Host 和 Client 当前都没有独立命令行子命令；运行 `wincast-host` 就读取默认配置并进入监听，运行 `wincast-client` 就读取默认配置并连接宿主端。当前已具备客户端有限重试、视频流中断重连和输入通道心跳超时识别；锁屏恢复编排和 Linux 真机长时间异常恢复仍需按烟测清单验证。
