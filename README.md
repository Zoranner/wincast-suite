# WinCast Suite

WinCast Suite 是一个 Rust 工具工程，用于在内网或专网内从国产化操作系统访问一台可被远程接管的 Windows 宿主机应用。

当前固定边界：

- Windows 宿主端以前台程序运行，读取本地配置并监听端口。
- 国产化 OS 客户端读取 IP 和端口配置并连接宿主端。
- 宿主端运行时链路负责启动配置中的 Windows 程序、捕获画面和注入输入。
- 客户端需要适配 Linux x86_64 与 Linux aarch64/ARM64。
- 部署上推荐使用专用账号自动登录，直接运行 Windows 宿主端可执行程序并保持常驻。
- 系统需要感知 Windows 登录、锁屏、解锁和注销状态；锁屏时暂停或断开会话，解锁后自动恢复或允许客户端重连。
- 远程登录/解锁、剪贴板同步、文件传输、多客户端并发、公网访问、NAT 穿透、UDP 媒体通道和 WebRTC 是永久非目标，不规划；系统也不处理 UAC 安全桌面，不提供不影响本地用户的独立远程会话。

当前代码已完成 Rust workspace、协议/配置模型、host/client 运行入口、控制消息编解码、TCP 直连握手、Windows 宿主端启动配置程序和主窗口定位入口、`wincast-capture` 捕获抽象、Linux 客户端 SDL2 显示入口、基础键鼠输入回传，以及基于 OpenH264 的 H.264 编码和解码。运行 `wincast-host` 会直接读取默认配置并进入持续监听；宿主端收到会话启动请求后会启动配置程序、定位主窗口、初始化捕获会话，并把捕获画面编码为 H.264 帧发送；Linux 客户端会解码 H.264 帧并渲染到窗口，把采集到的基础键鼠事件写回同一条 TCP 连接，并在窗口退出时发送 `StopSession`；Windows 宿主端已接入基础 SendInput 输入注入。当前正式设计和示例配置口径固定为低延迟 H.264 编码视频流，捕获模式收敛为 `capture.mode = "auto" | "window" | "display"`：`auto` 优先窗口捕获，窗口捕获失败或全屏程序黑屏时使用唯一显示器捕获兜底；当前只面向单显示器，画面上限为 1920x1080，并按宿主实际画面走，不主动降采样。锁屏恢复和会话自动重连仍未完成。

配置文件默认从用户配置目录读取：Windows host 默认读取 `%APPDATA%\WinCast\wincast-host.toml`，Linux client 默认读取 `${XDG_CONFIG_HOME:-$HOME/.config}/wincast/wincast-client.toml`。`XDG_CONFIG_HOME` 必须是非空绝对路径；未设置、为空或为相对路径时回退到 `$HOME/.config`。两端运行各自可执行程序即可启动；`examples/` 目录提供稳定版烟测示例配置。

设计文档见 [docs/design.md](docs/design.md)，开发说明见 [docs/development.md](docs/development.md)，稳定版真机烟测清单见 [docs/smoke-test.md](docs/smoke-test.md)。
