# WinCast Suite

WinCast Suite 是一个 Rust 工具工程，用于在内网或专网内从国产化操作系统访问一台可被远程接管的 Windows 宿主机应用。

当前固定边界：

- Windows 宿主端以前台程序运行，读取本地配置并监听端口。
- 国产化 OS 客户端读取 IP 和端口配置并连接宿主端。
- 宿主端运行时链路负责启动配置中的 Windows 程序、捕获画面和注入输入。
- 客户端需要适配 Linux x86_64 与 Linux aarch64/ARM64。
- 系统不处理 Windows 登录、锁屏、UAC 安全桌面或不影响本地用户的独立远程会话。

当前代码已完成 Rust workspace、协议/配置模型、host/client CLI 骨架、控制消息编解码、最小 TCP 控制通道握手、Windows 宿主端启动配置程序和主窗口定位入口，并新增 `wincast-capture` 捕获抽象和 SDL2 raw BGRA 渲染后端。CLI `run` 可以建立宿主端连接并完成 `Hello` / `StartSession` 控制消息交换；宿主端收到会话启动请求后会尝试启动配置程序、定位主窗口、通过 Windows Graphics Capture 初始化捕获会话、等待首帧 BGRA readback 缓冲，随后发送 `SessionReady`、`VideoReady` 和 raw BGRA 二进制帧；Linux 客户端会创建 SDL2 窗口渲染 raw BGRA 首帧。当前主线优先打通低复杂度 raw 帧链路，H.264/WebRTC 编码传输作为后续性能优化项；长时间渲染循环、输入事件发送和输入注入仍未实现。

设计文档见 [docs/design.md](docs/design.md)。
