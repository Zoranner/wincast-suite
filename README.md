# WinCast Suite

WinCast Suite 是一个 Rust 工具工程，用于在内网或专网内从国产化操作系统访问一台可被远程接管的 Windows 宿主机应用。

当前固定边界：

- Windows 宿主端以前台程序运行，读取本地配置并监听端口。
- 国产化 OS 客户端读取 IP 和端口配置并连接宿主端。
- 宿主端运行时链路负责启动配置中的 Windows 程序、捕获画面和注入输入。
- 客户端需要适配 Linux x86_64 与 Linux aarch64/ARM64。
- 部署上推荐使用专用账号自动登录，后续由 Windows Service 拉起交互桌面内的 Host Agent。
- 系统需要感知 Windows 登录、锁屏、解锁和注销状态；锁屏时暂停或断开会话，解锁后自动恢复或允许客户端重连。
- 远程登录/解锁、剪贴板同步、文件传输和多客户端并发是永久非目标，不规划；系统也不处理 UAC 安全桌面，不提供不影响本地用户的独立远程会话。

当前代码已完成 Rust workspace、协议/配置模型、host/client CLI 骨架、控制消息编解码、最小 TCP 控制通道握手、Windows 宿主端启动配置程序和主窗口定位入口，并新增 `wincast-capture` 捕获抽象和 SDL2 raw BGRA 渲染后端。CLI `run` 可以建立宿主端连接并完成 `Hello` / `StartSession` 控制消息交换；宿主端收到会话启动请求后会尝试启动配置程序、定位主窗口、通过 Windows Graphics Capture 初始化捕获会话、等待首帧 BGRA readback 缓冲，随后发送 `SessionReady`、`VideoReady` 并持续写入 raw BGRA 二进制帧；Linux 客户端会创建 SDL2 窗口持续渲染 raw BGRA 帧，把 SDL2 采集到的基础键鼠事件写回控制连接，并在窗口退出时发送 `StopSession`；Windows 宿主端已接入基础 SendInput 输入注入。当前主线的默认可用画面链路是 raw BGRA，窗口捕获是稳定烟测默认路径，`capture.mode = "desktop"` 已支持捕获目标窗口所在显示器但仍需要真机验证多屏和性能边界。H.264/WebRTC 编码传输仍是后续性能优化项，现阶段不应视为默认可用能力。协议与配置层虽然已经预留 `VideoCodec::H264`、`EncodedVideoFrame` 等扩展点，但当前运行期还没有接入 H.264 编码器、编码帧发送或 WebRTC 传输。Service/Agent IPC 已完成长度前缀 JSON frame 传输底座、Host 侧通用 Read/Write endpoint、消息模型测试、最小 TCP loopback transport round-trip，以及 Service 侧 Agent 状态查询和会话启动/停止 request/ack coordinator；客户端 `run` 已支持 `--retries` 与 `--retry-delay-ms` 做有限连接重试；`service` 命令已接入 Windows SCM 的安装、卸载、启动、停止和状态查询最小闭环，但 Service 拉起交互桌面 Host Agent、命名管道权限模型、锁屏恢复和会话自动重连仍未完成。

配置文件默认从用户配置目录读取，日常运行不需要每次传 `--config`：Windows host 默认读取 `%APPDATA%\WinCast\wincast-host.toml`，Linux client 默认读取 `${XDG_CONFIG_HOME:-$HOME/.config}/wincast/wincast-client.toml`。`XDG_CONFIG_HOME` 必须是非空绝对路径；未设置、为空或为相对路径时回退到 `$HOME/.config`。`examples/` 目录提供稳定版烟测示例配置，`--config` 仅用于临时调试或一次性验证时覆盖默认路径。

设计文档见 [docs/design.md](docs/design.md)，开发说明见 [docs/development.md](docs/development.md)，稳定版真机烟测清单见 [docs/smoke-test.md](docs/smoke-test.md)。
