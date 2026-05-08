# WinCast Suite

WinCast Suite 是一个 Rust 工具工程，用于在内网或专网内从国产化操作系统访问一台可被远程接管的 Windows 宿主机应用。

当前固定边界：

- Windows 宿主端以前台程序运行，读取本地配置并监听端口。
- 国产化 OS 客户端读取 IP 和端口配置并连接宿主端。
- 宿主端运行时链路负责启动配置中的 Windows 程序、捕获画面和注入输入。
- 客户端需要适配 Linux x86_64 与 Linux aarch64/ARM64。
- 系统不处理 Windows 登录、锁屏、UAC 安全桌面或不影响本地用户的独立远程会话。

当前代码已完成 Rust workspace、协议/配置模型、host/client CLI 骨架、控制消息编解码和最小 TCP 控制通道握手。CLI `run` 可以建立宿主端连接并完成 `Hello` / `StartSession` 控制消息交换，但仍会明确暴露“运行时链路未实现”，不会假装程序启动、捕获、编码、传输、渲染或输入注入已经可用。

设计文档见 [docs/design.md](docs/design.md)。
