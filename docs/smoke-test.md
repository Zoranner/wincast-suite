# 稳定版真机烟测清单

本清单覆盖稳定版真机验收口径：Windows 宿主端可执行程序常驻运行、`capture.mode = "auto"`、窗口捕获优先、全屏程序或窗口捕获失败时使用唯一显示器捕获兜底、低延迟 H.264 编码传输、Linux SDL2 客户端显示与基础键鼠输入。不要把通过项外推为锁屏恢复或 ARM64 真机窗口运行已经完成。交叉编译和 Windows 开发机上的 workspace 测试不能替代 Linux x86_64 或 Linux aarch64/ARM64 真机验证。

## 准备环境

- Windows host 与 Linux client 位于同一可达网络，Windows 防火墙已放通 Host 监听端口。
- Windows host 使用可交互桌面登录，待捕获应用能以前台程序启动，并且目标窗口标题包含稳定可匹配文本。
- Linux client 已安装 C/C++ 编译工具链、`pkg-config` 和 SDL2 运行/开发依赖；OpenH264 后端会在构建时编译 C/C++ 源码。ARM64 目标机需要在真实 aarch64/ARM64 Linux 设备上执行同一流程。
- 两端使用同一版本产物，Host 使用 `wincast-host`，Client 使用 `wincast-client`。

## 配置

配置文件以仓库内 `examples/` 目录为准，烟测时复制示例文件到默认用户配置目录后按现场环境调整，避免在清单里维护第二份 TOML。Host 运行入口只读取默认配置目录。

- Windows host 复制 `examples/wincast-host.toml` 为 `%APPDATA%\WinCast\wincast-host.toml`。
- Linux client 复制 `examples/wincast-client.toml` 为 `${XDG_CONFIG_HOME:-$HOME/.config}/wincast/wincast-client.toml`。`XDG_CONFIG_HOME` 必须是非空绝对路径；未设置、为空或为相对路径时使用 `$HOME/.config/wincast/wincast-client.toml`。

Windows host 使用 H.264 编码链路和自动捕获策略，重点核对以下字段：

- `listen`：Host 监听地址，默认端口需与 Client 的 `port` 一致。
- `program`、`args`、`work_dir`：待捕获应用的启动命令和工作目录。
- `video.codec`：稳定版烟测使用 `h264`。
- `video.width`、`video.height`：目标上限最高 1920x1080，按宿主实际画面走，不主动降采样。
- `capture.mode`：稳定版默认烟测使用 `auto`；`window` 表示只验证普通窗口捕获，`display` 表示捕获唯一宿主显示器。
- `capture.window_title_contains`：用于普通窗口定位；全屏程序或窗口捕获失败时，由 `auto` 模式切到唯一显示器捕获兜底。
- `capture.startup_timeout_ms`：目标应用启动和窗口出现的等待时间。

Linux client 指向 Windows host，重点核对以下字段：

- `host`：Windows host 的可达 IP 或主机名。
- `port`：必须与 Host 配置中的 `listen` 端口一致。

## 执行步骤

- 在 Windows host 上执行 `wincast-host`，确认控制通道进入持续监听。
- 在 Linux client 上执行 `wincast-client`，确认能建立连接并打开 SDL2 窗口。
- 客户端内置启动连接重试，只覆盖初始连接阶段，不覆盖会话建立后的链路中断自动恢复；中断后需要重新启动客户端或按现场流程重新连接。
- 观察客户端窗口，确认能看到 Windows 目标应用窗口画面，且窗口移动或目标应用内容变化后客户端画面随之更新。
- 在客户端窗口内移动鼠标、点击、滚轮滚动，并在目标应用可输入区域敲入普通字符，确认 Windows 目标应用收到鼠标和键盘输入。
- 关闭 Linux 客户端 SDL2 窗口，确认客户端退出时发送 `StopSession`，Windows host 结束当前会话并清理捕获/输入链路。
- 不重启 Windows host，再次启动 Linux client 连接同一 host，确认 host 能接受下一次连接并重新看到画面。
- 可选：将 Windows host 的 `capture.mode` 改为 `window` 后重复宿主端运行流程，确认普通窗口捕获路径仍可用。
- 可选：将 Windows host 的 `capture.mode` 改为 `display` 后重复宿主端运行流程，确认客户端显示唯一宿主显示器画面，且全屏程序可通过该路径兜底。

## Linux 真机验证入口

Linux x86_64 和 Linux aarch64/ARM64 都需要在对应真机上按本清单执行客户端流程。x86_64 真机验证覆盖常见 Linux 桌面运行路径；aarch64/ARM64 真机验证覆盖目标架构上的 SDL2 链接、窗口创建、渲染和输入回传。交叉编译检查只能作为编译边界补充，不能写作 ARM64 真机通过。

## 通过标准

- Host 直接运行 `wincast-host` 后保持监听，`capture.mode = "auto"` 能进入 H.264 视频会话。
- Linux client 能连接 host，打开窗口并持续显示目标应用画面。
- 鼠标移动、点击、滚轮和普通键盘输入能从客户端回传到 Windows 目标应用。
- 客户端关闭窗口后会话结束，host 没有卡死在旧会话，能接受下一次客户端连接。

## 当前不支持与永久非目标边界

- 锁屏、解锁后的自动恢复或自动重连编排仍未实现。
- 远程登录/解锁、剪贴板同步、文件传输、多客户端并发、公网访问、NAT 穿透、UDP 媒体通道和 WebRTC 是永久非目标，不规划。
- 稳定版烟测以 TCP 直连上的 H.264 编码帧传输为准。
- ARM64 Linux 客户端虽然有交叉编译边界，但 SDL2 链接、窗口创建、渲染和输入回传仍必须在目标 ARM64 真机上验证。
