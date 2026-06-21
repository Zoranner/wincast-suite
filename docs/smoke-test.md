# 稳定版真机烟测清单

本清单覆盖稳定版真机验收口径：Windows 宿主端可执行程序常驻运行、收到 Linux 客户端会话请求后启动配置程序、延迟后捕获当前整块屏幕、低延迟 H.264 编码传输、Linux SDL2 客户端显示与基础键鼠输入。执行本清单前，必须先完成 [部署准备说明](deployment-prep.md) 中的产物、配置、依赖和防火墙检查。不要把通过项外推为锁屏恢复、ARM64 真机运行或 Unity embedded 端到端已经完成。交叉编译、Windows 开发机上的 workspace 测试和 Unity package 静态接入检查不能替代 Linux x86_64、Linux aarch64/ARM64 或 Unity Player 真机验证。

默认清单仍适用于 `desktop_dxgi` 普通模式。Unity embedded 模式的烟测项单独列出，当前文档只是定义真机验证前后应如何执行和记录，不表示 Unity Player、native DLL 加载或 Linux client 到 Unity Player 的链路已经验证通过。

## 准备环境

- Windows host 与 Linux client 位于同一可达网络，Windows 防火墙已放通本次模式需要的监听端口。
- Windows host 使用可交互桌面登录，待启动应用能以前台全屏程序运行。
- Windows host 使用单显示器整屏捕获，屏幕上出现的系统弹窗、任务栏或其他窗口都会被传输。
- Linux client 已安装 C/C++ 编译工具链、`pkg-config` 和 SDL2 运行/开发依赖；OpenH264 后端会在构建时编译 C/C++ 源码。ARM64 目标机需要在真实 aarch64/ARM64 Linux 设备上执行同一流程。
- 两端使用同一版本产物，Host 使用 `wincast-host`，Client 使用 `wincast-client`。

## 配置

配置文件以仓库内 `examples/` 目录为准，烟测时复制示例文件到默认用户配置目录后按现场环境调整，避免在清单里维护第二份 TOML。Host 运行入口只读取默认配置目录。

- Windows host 复制 `examples/host.toml` 为 `%APPDATA%\WinCast\host.toml`。
- Linux client 复制 `examples/client.toml` 为 `${XDG_CONFIG_HOME:-$HOME/.config}/wincast/client.toml`。`XDG_CONFIG_HOME` 必须是非空绝对路径；未设置、为空或为相对路径时使用 `$HOME/.config/wincast/client.toml`。

Windows host 使用 H.264 编码链路和整屏捕获策略，重点核对以下字段：

- `listen`：Host 监听地址，默认端口需与 Client 的 `port` 一致。
- `program.path`、`program.args`、`program.work_dir`：待启动应用的启动命令和工作目录。
- `program.startup_delay_ms`：启动程序后延迟多久开始整屏捕获；延迟期间关闭客户端也应立即结束本次会话并清理宿主端程序。
- `video.codec`：稳定版烟测使用 `h264`。
- `video.width`、`video.height`：H.264 编码输出尺寸，最高 1920x1080；宿主端整屏捕获画面会按该尺寸缩放后传输。
- `capture.first_frame_timeout_ms`：整屏捕获启动后等待首帧的保护超时。

Linux client 指向 Windows 侧实际远控服务，重点核对以下字段：

- `host`：Windows host 的可达 IP 或主机名。
- `port`：普通 `desktop_dxgi` 模式必须与 Host 配置中的 `listen` 端口一致；Unity embedded 模式必须与 Host 配置中的 `[unity].port` 一致。

## 普通模式执行步骤

- 在 Windows host 上执行 `wincast-host`，确认控制通道进入持续监听。
- 在 Linux client 上执行或双击启动 `wincast-client`，确认客户端立即进入 SDL2 全屏窗口并显示加载进度。
- 客户端内置有限重试，覆盖初始连接失败、宿主端会话门禁的可恢复拒绝和 H.264 视频流中断；烟测时仍需观察重试次数耗尽后的错误提示，不能把它外推为锁屏/解锁恢复已经完成。
- 观察客户端窗口，确认能看到 Windows 宿主机整块屏幕，且目标应用画面变化后客户端画面随之更新。
- 在客户端窗口内移动鼠标、点击、滚轮滚动，并在目标应用可输入区域敲入普通字符，确认 Windows 目标应用收到鼠标和键盘输入。
- 关闭 Linux 客户端 SDL2 窗口，确认客户端退出时发送 `StopSession`，Windows host 结束当前会话并清理捕获、输入链路和本次启动的程序树。
- 在会话期间手动关闭目标应用，确认客户端把宿主端程序退出视为正常会话结束并退出，并且 Windows host 能接受下一次连接。
- 不重启 Windows host，再次启动 Linux client 连接同一 host，确认 host 能接受下一次连接并重新看到画面。

## Unity embedded 烟测准备与执行口径

Unity embedded 真机烟测必须在真实 Unity Player 产物上执行。开始前确认：

- Unity Player 已接入 `unity/com.zoranner.wincast` package，场景中挂载并启用 `WinCastUnityAgent`。
- `wincast_unity_native.dll` 已放入 Unity 插件目录，目标 Player 架构与 DLL 架构一致。
- Host 配置包含 `mode = "unity_embedded"` 和 `[unity]`，其中 `[unity].executable`、`[unity].work_dir` 和 `[unity].port` 指向现场 Player。
- Host 只向 Unity Player 传入 `--wincast-port <port>`；分辨率、FPS、码率和输入适配不由 Host 启动参数覆盖。
- Linux client 配置的 `host` 指向 Windows 机器，`port` 指向 `[unity].port`，不是普通模式的 Host `listen` 端口。
- Windows 防火墙已开放 `[unity].port`，并按现场网络边界限制来源 IP 或网段。

Unity embedded 真机烟测执行时，按以下结果记录，不得用准备项替代通过项：

- 启动 `wincast-host` 后，确认 Host 拉起 Unity Player，并在日志或现场观察中记录传入的 `--wincast-port` 端口。
- 在 Linux client 上启动 `wincast-client`，确认连接的是 Unity Player 的 `[unity].port`。
- 确认客户端收到首帧并显示 Unity Player 最终 Game View 画面，而不是桌面 DXGI 整屏画面。
- 在客户端窗口执行鼠标和键盘输入，确认 Unity UI 或业务输入适配层收到事件。
- 关闭 Linux client，确认 Unity Player 内 native runtime 释放客户端会话，Player 本身的生命周期符合现场预期。
- 关闭 Unity Player 后，确认 Host 能感知进程退出并释放本次监督流程。

## Linux 真机验证入口

Linux x86_64 和 Linux aarch64/ARM64 都需要在对应真机上按本清单执行客户端流程。x86_64 真机验证覆盖常见 Linux 桌面运行路径；aarch64/ARM64 真机验证覆盖目标架构上的 SDL2 链接、窗口创建、渲染和输入回传。交叉编译检查只能作为编译边界补充，不能写作 ARM64 真机通过。

## 通过标准

- Host 直接运行 `wincast-host` 后保持监听，并能在 Linux client 发起会话后进入 H.264 视频会话。
- Linux client 能连接 host，先打开全屏加载窗口，首帧到达后持续显示 Windows 宿主机整屏画面。
- 鼠标移动、点击、滚轮和普通键盘输入能从客户端回传到 Windows 目标应用。
- 客户端关闭窗口后会话结束，host 没有卡死在旧会话，能接受下一次客户端连接。

Unity embedded 模式只有在真实 Unity Player 上额外满足以下条件，才能记录为该模式通过：

- Unity Player 运行时成功加载 `wincast_unity_native.dll`。
- Linux client 连接 `[unity].port` 后收到 Unity 最终画面帧。
- 鼠标和键盘输入经 Unity package 分发到 UI 或业务适配层。
- Host 没有把分辨率、FPS、码率或鉴权参数作为 Unity 启动参数传入，现场记录中只出现 `--wincast-port`。

## 当前不支持与永久非目标边界

- 锁屏、解锁后的完整自动恢复编排仍未完成，不能写作已通过真机验证。
- 远程登录/解锁、剪贴板同步、文件传输、多客户端并发、公网访问、NAT 穿透、UDP 媒体通道和 WebRTC 是永久非目标，不规划。
- 稳定版烟测以 TCP 直连上的 H.264 编码帧传输为准。
- ARM64 Linux 客户端虽然有交叉编译边界，但 SDL2 链接、窗口创建、渲染和输入回传仍必须在目标 ARM64 真机上验证。
- Unity embedded 准备项不能写作 Unity Player 真机验证通过；必须记录真实 Player、native DLL、端口、防火墙、Linux client 和输入回传结果。
