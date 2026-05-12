# WinCast Suite 设计方案

## 项目定位

WinCast Suite 面向国产化操作系统访问 Windows 应用的内网远程应用工具。系统采用“专用 Windows 宿主机可被远程接管”的边界：Linux 客户端连接 Windows 宿主端后，由宿主端启动配置好的 Windows 程序，采集该程序窗口或桌面区域画面并传输给客户端，同时把客户端鼠标和键盘事件映射回 Windows 宿主机。客户端明确面向 Linux x86_64 与 Linux aarch64/ARM64 两类部署目标。

本项目使用 Rust 开发，优先保证链路清晰、部署简单、低延迟可验证。系统不追求替代 RDP/RemoteApp，也不承诺不影响宿主机本地使用。

## 使用边界

系统只支持内网或专网直连。客户端直接配置 Windows 宿主机 IP 地址和端口，宿主端直接配置监听端口、程序路径、启动参数、工作目录、画面参数等基础项。

Windows 宿主机需要满足以下前提：

- 推荐使用专用账号自动登录，避免宿主机重启后停在登录界面。
- 已登录到一个可交互桌面。
- 桌面未锁屏；锁屏期间远程会话应暂停或断开，不能继续捕获和注入输入。
- 推荐接入物理显示器、虚拟显示器或 HDMI 虚拟头，以保证无人值守时仍有稳定分辨率。
- 被启动程序不应依赖 UAC 安全桌面交互。
- 宿主机本地若有人同时使用，会被远程输入和窗口焦点影响。

系统支持通过部署策略让专用账号自动登录，并在运行时感知登录、锁屏、解锁和注销状态。系统不做远程输入 Windows 凭据解锁，不捕获或控制登录界面、锁屏界面、UAC 安全桌面、独立多用户远程会话，也不保证本地用户无感使用。

## 目标能力

- Linux x86_64 与 Linux aarch64/ARM64 客户端通过 IP 和端口连接 Windows 宿主端。
- Windows 宿主端根据配置启动一个指定程序。
- 宿主端定位目标程序窗口，捕获窗口画面；窗口捕获不稳定时允许退回桌面区域捕获。
- 宿主端以低延迟视频流方式传输画面。
- 客户端展示远程画面，并采集鼠标、键盘、滚轮事件。
- 宿主端接收输入事件并注入到 Windows 交互桌面。
- 连接断开后释放捕获、编码、进程和网络资源。
- 对启动失败、找不到窗口、捕获失败、连接断开等基础错误给出明确提示。
- Windows 宿主端开机后能够通过服务进程进入可等待连接状态。
- 已登录且未锁屏时自动拉起交互桌面内的 Host Agent。
- 检测到锁屏、注销或交互桌面不可用时暂停或断开会话，并向客户端返回明确状态。
- 解锁或重新登录后恢复 Host Agent，允许客户端自动重连或手动重连。

## 非目标能力

系统明确不做以下能力：

- 白名单、客户端证书、用户权限、审计日志。
- 空闲超时、崩溃重启、单实例或多实例策略。
- 敏感组合键过滤。
- 文件传输、剪贴板同步、音频、打印、USB 重定向。
- 管理端、服务发现、自动注册、中心调度。
- 公网访问、NAT 穿透、TURN 中继。
- 不影响宿主机本地用户的独立远程会话。
- 远程输入 Windows 账号密码来登录或解锁。
- 对 Windows 登录界面、锁屏界面、UAC 安全桌面的捕获和控制。
- Credential Provider、自定义登录凭据入口或替代 Windows 登录界面。

## 总体架构

系统由 Windows 宿主端和国产化操作系统客户端两部分组成。两端共享一套 Rust 协议 crate，保证控制消息、输入事件、错误码和配置模型一致。

```text
Linux 客户端
  -> 读取 host/port
  -> 建立内网连接
  -> 创建本地显示窗口
  -> 接收并渲染远程画面
  -> 采集鼠标键盘事件并发送

Windows 宿主端
  -> 服务进程开机自启并读取监听端口和程序配置
  -> 检测当前用户会话、锁屏和解锁状态
  -> 在交互桌面内拉起 Host Agent
  -> 等待客户端连接
  -> 启动 Windows 程序
  -> 定位目标窗口
  -> 捕获窗口或桌面区域
  -> 发送画面帧
  -> 接收输入事件并注入
```

宿主端同一时间只处理一个客户端连接。若已有客户端连接，新的连接应返回“忙碌”错误并关闭。

## Rust 工程结构

当前 Cargo workspace：

```text
crates/
  wincast-protocol/    # 控制消息、输入事件、配置模型、错误码、Service/Agent IPC 消息模型
  wincast-host/        # Windows 宿主端主程序
  wincast-capture/     # Windows 画面捕获封装
  wincast-input/       # Windows 输入注入封装
  wincast-client/      # Linux x86_64 与 Linux aarch64/ARM64 客户端主程序
  wincast-render/      # 客户端 raw BGRA 渲染封装，当前使用 SDL2
docs/
  design.md
```

各 crate 边界如下：

- `wincast-protocol` 不依赖平台 API，只定义可序列化的数据结构；当前已包含 Service 与 Host Agent 的 IPC 消息模型和长度前缀 JSON frame 编解码底座，并通过 JSON round-trip 与 frame 传输测试约束序列化边界。
- `wincast-host` 负责配置读取、连接管理、进程生命周期和各模块编排；当前已包含登录、锁屏和 Agent 可用性的纯状态机模型、平台事件映射边界，以及给定窗口句柄后的 Windows 会话通知注册/注销边界，但尚未接入消息循环、真实状态检测和恢复编排。
- `wincast-capture` 只封装 Windows 捕获能力，不处理网络和进程启动。
- `wincast-input` 只封装 Windows 输入注入，不理解客户端 UI、网络会话或窗口生命周期。
- `wincast-client` 负责连接宿主端、窗口生命周期和本地事件采集。
- `wincast-render` 负责帧缓冲和渲染输出；当前使用 SDL2 在 Linux 上显示 raw BGRA 帧，后续接入 H.264 时再承担视频解码。

## 当前 CLI 骨架

当前 `wincast-host` 与 `wincast-client` 已在配置读取、配置校验和 TCP 控制通道握手基础上接入 raw BGRA 捕获传输、SDL2 渲染和基础输入回传。若宿主端提示编码传输未实现，应理解为 H.264/WebRTC 等编码传输路线尚未接入，不代表当前 raw BGRA 链路不可用；当前 `run` 的默认可用画面链路就是 raw BGRA。

会话状态与 Service/Agent 分层当前仍是底座能力：`wincast-host` 已新增纯状态机，用于表达未登录、已登录、锁屏、Agent 可用、会话运行、会话暂停和错误状态之间的转换，并补齐平台事件到状态机事件的映射边界；`wincast-protocol` 已新增 Service 与 Host Agent 的 IPC 消息模型和长度前缀 JSON frame 编解码，用于表达 Agent 在线状态、启动会话、结束会话、锁屏通知和错误上报；`wincast-host` 已有可测试的通用 Read/Write IPC endpoint，并新增最小 TCP loopback transport，用于复用上述 frame 完成 Service/Agent 双向 round-trip；Service 侧已具备最小 Agent coordinator，可通过 `QueryStatus` 获取 Agent 上报状态，并对 `StartSession` / `StopSession` 做 request/ack 校验；前台 Host Agent 运行入口也已从 CLI 命令分发中抽出，便于后续 Service 复用。上述能力目前只约束内存状态、可序列化消息、通用 frame 传输、本机 loopback 连接、Service 侧 request/ack coordinator 和 Agent runtime 边界，不代表已经实现 Windows API 监听、真实 Windows Service 或 Service 拉起 Host Agent。

宿主端 CLI：

```text
wincast-host
wincast-host validate
wincast-host run
wincast-host service install
wincast-host service uninstall
wincast-host service start
wincast-host service stop
wincast-host service status
```

客户端 CLI：

```text
wincast-client
wincast-client validate
wincast-client run
wincast-client run --retries 3 --retry-delay-ms 1000
wincast-client targets
```

不带子命令时默认进入 `run`。Host 与 Client 默认从用户配置目录读取配置：Windows host 默认读取 `%APPDATA%\WinCast\wincast-host.toml`，Linux client 默认读取 `${XDG_CONFIG_HOME:-$HOME/.config}/wincast/wincast-client.toml`。`XDG_CONFIG_HOME` 必须是非空绝对路径；未设置、为空或为相对路径时回退到 `$HOME/.config`。`--config` 仅用于临时调试或一次性验证时覆盖默认路径。宿主端 `run` 在配置校验通过后进入持续 TCP 监听，同一时间只处理一个客户端会话；空闲时接受客户端 `Hello` 和 `StartSession` 控制消息，随后尝试启动配置程序、定位主窗口、通过 Windows Graphics Capture 初始化捕获会话、等待首帧 BGRA readback 缓冲，发送 `SessionReady`、`VideoReady` 并持续写入 raw BGRA 二进制帧；已有会话运行时，新的客户端连接会收到忙碌错误。`service` 子命令当前通过可测试的 `ServiceManager` 管理抽象返回占位结果，用于先固定 CLI 与管理边界，不会执行真实 Windows Service 安装、卸载、启动、停止或状态查询。Linux 客户端 `run` 连接宿主端、发送 `Hello` 和 `StartSession`，创建 SDL2 窗口持续渲染 raw BGRA 帧，轮询 SDL2 基础键鼠事件并写回控制连接，窗口退出时发送 `StopSession`；`--retries` 和 `--retry-delay-ms` 只覆盖启动连接阶段的有限重试，不改变会话中断后的恢复语义，也不代表 Service/Agent 自动恢复已经完成。宿主端使用阻塞输入读取线程处理客户端输入事件，避免在非阻塞单次 `read_message` 中丢失半包状态，并通过 Windows SendInput 注入基础鼠标、滚轮和键盘事件。非 Linux 开发环境只执行协议校验路径，并把宿主端错误响应明确暴露出来。当前 `wincast-protocol` 已定义 raw BGRA 二进制帧、`VideoReady`、Service/Agent IPC 长度前缀 JSON frame 和后续可选 H.264 `EncodedVideoFrame` 线格式；当前主线优先打通 raw BGRA 帧链路，H.264/WebRTC 只作为后续性能优化项。`wincast-capture` 已接入 WGC 支持检测、窗口捕获目标创建、D3D11 设备、帧池、捕获会话启动、首帧等待、帧元数据读取、D3D11 纹理描述读取、尺寸变化后的帧池重建和可选 BGRA readback；`wincast-render` 已提供 SDL2 raw BGRA 窗口后端。客户端 `targets` 必须明确列出 `x86_64-unknown-linux-gnu` 与 `aarch64-unknown-linux-gnu`，对应 Linux x86_64 与 Linux aarch64/ARM64。

## 宿主端设计

当前代码中的 `wincast-host` 仍是 Windows 前台可执行程序，方便先把启动、窗口定位、捕获、输入和传输链路打通。当前已补齐会话状态纯模型、平台事件映射边界、Service/Agent IPC 消息模型、长度前缀 JSON frame 底座、Host 侧通用 IPC endpoint 和可测试的 ServiceManager 占位抽象；后续为了处理开机自启、登录态和锁屏状态，宿主端仍需要演进为 Windows Service 加用户态 Host Agent 的两层结构。

Windows Service 负责：

- 开机自启动并读取全局配置。
- 监听客户端连接或协调监听端口占用。
- 订阅 Windows 会话状态变化，识别登录、锁屏、解锁和注销。
- 在已登录且未锁屏的交互桌面内拉起 Host Agent。
- 在锁屏、注销或 Agent 退出时清理会话状态，并向客户端返回明确错误或状态。

Host Agent 负责：

- 运行在用户交互桌面内，不运行在 Session 0。
- 启动配置程序、定位目标窗口、建立捕获会话。
- 发送画面帧并接收输入事件。
- 使用 Windows 输入注入能力作用到当前交互桌面。
- 与 Service 通过本机 IPC 交换状态和控制命令。

Service 不直接做窗口捕获和输入注入，避免 Session 0 与交互桌面隔离导致能力不可用或行为不可预测。

当前尚未实现 Windows Service 安装、卸载、启动、停止和开机自启，也尚未实现 Service 拉起 Host Agent、Agent 保活、命名管道权限模型、重连心跳或 Service 侧端口编排。现有 IPC 类型、长度前缀 JSON frame、最小 TCP loopback transport 和 Service 侧 request/ack coordinator 只是 Service 与 Agent 之间通信的底座，ServiceManager 也仍是可测试占位抽象。

宿主端启动流程：

```text
读取配置
-> 检查当前用户会话状态
-> 已登录且未锁屏时启动 Host Agent
-> 监听 TCP 端口
-> 接收客户端连接
-> 启动配置程序
-> 等待程序进入可交互状态
-> 枚举进程窗口并选择主窗口
-> 建立捕获会话
-> 建立视频发送循环
-> 建立输入接收循环
```

程序启动使用 Windows 进程 API。窗口定位优先按进程 ID 枚举顶层窗口，过滤不可见窗口、工具窗口和尺寸异常窗口。如果目标程序会先启动 launcher 再拉起子进程，可以通过配置项补充窗口标题关键字或等待时间。

## 登录与锁屏处理

登录和锁屏处理以“感知状态、自动恢复、不远程提交 Windows 凭据”为原则。宿主机部署时推荐启用专用账号自动登录，解决断电重启、系统更新重启后无人值守恢复的问题。自动登录属于部署策略，不等同于远程解锁；其安全边界需要由专用账号权限、物理访问限制和内网隔离共同约束。

系统运行时需要区分以下状态：

- 未登录：Service 保持运行并等待用户会话出现；客户端连接时返回宿主机未登录状态。
- 已登录且未锁屏：Service 拉起或保持 Host Agent，允许建立远程会话。
- 已锁屏：停止新的会话启动；已有会话暂停或断开，客户端显示宿主机已锁定。
- 解锁后：Service 重新确认交互桌面可用，拉起或恢复 Host Agent，客户端可自动重连或手动重连。
- 注销或切换用户：释放 Agent、捕获、输入和会话资源，客户端收到会话结束或状态错误。

当前代码已用纯状态机描述这些状态转换和对客户端可见的状态/错误，并补齐了 `WTSRegisterSessionNotification` / `WTSUnRegisterSessionNotification` 的注册边界，用于后续在拥有窗口句柄和消息循环后接收 `WM_WTSSESSION_CHANGE`。但当前尚未接入 `WTSGetActiveConsoleSessionId`、桌面锁定事件分发或其他 Windows 会话状态检测，也没有实现消息循环到状态机的运行时编排；现阶段仍不做真实状态恢复。系统也不做 Credential Provider，不采集、不保存、不转发 Windows 登录密码。若未来确实需要远程登录或远程解锁，应作为独立安全敏感模块评估，不并入当前 raw BGRA 主线。

## 画面捕获

Windows 捕获优先采用窗口级捕获，目标是只传输被启动程序的窗口画面。实现上优先调研并封装 Windows Graphics Capture；当窗口级捕获在特定系统或应用上不稳定时，允许使用桌面捕获加窗口区域裁剪作为兜底。

捕获模块需要处理：

- DPI 缩放导致的逻辑坐标与物理像素坐标差异。
- 窗口移动、缩放、最小化、关闭。
- 主窗口句柄重建。
- 弹窗和子窗口是否纳入画面。
- 捕获失败后的错误上报和资源释放。

推荐固定目标为 1280x720 或 1920x1080、30 FPS。画面尺寸由宿主端配置决定，客户端按收到的视频尺寸创建或调整显示区域。

## 视频传输

当前阶段优先使用低复杂度 raw BGRA 帧通道，先形成可见画面的端到端闭环。该路线参考“覆盖式最新帧”模型：宿主端捕获 BGRA 帧后写入独立二进制帧，客户端读取最新帧并渲染；慢客户端允许丢帧，不把每一帧都堆成可靠队列。

raw BGRA 帧头包含 magic、宽度、高度、row pitch、序号、时间戳和 payload 长度，payload 为 BGRA32 字节。控制消息只承载握手、阶段切换、错误和输入事件，不承载持续大帧。

客户端当前窗口后端固定为 SDL2，不引入 Slint 或 egui。这样可以降低银河麒麟 V10 等老系统上的 GPU、Vulkan、Wayland 和桌面环境依赖风险，优先通过 SDL2 streaming texture 直接显示 BGRA8888 帧。

H.264/WebRTC 保留为后续优化路线：当 raw BGRA 在目标分辨率、帧率或网络环境下不可接受时，再接入 H.264 编码、媒体传输和解码渲染。内网场景不需要公网 TURN 中继，连接模型限制为局域网直连。

Rust 负责配置、生命周期和协议编排。后续媒体底层可以封装 Rust WebRTC 能力；如果硬件编码和系统媒体管线集成成本过高，也可以封装 GStreamer 管线，但外部架构和协议边界保持不变。

## 输入映射

客户端采集本地窗口内的鼠标和键盘事件，转换为协议事件发送给宿主端。鼠标坐标使用归一化坐标或远程画面像素坐标，宿主端根据当前捕获区域映射回 Windows 桌面坐标。

输入事件类型：

- 鼠标移动。
- 鼠标按下和释放。
- 鼠标滚轮。
- 键盘按下和释放。
- 修饰键状态。

宿主端使用 Windows 输入注入能力把事件注入到当前交互桌面。该方式会影响宿主机本地鼠标、键盘和窗口焦点，这是工具边界的一部分。

系统不实现完整远程输入法。中文输入依赖 Windows 端输入法状态和按键事件。

## 协议设计

控制消息使用长度前缀二进制消息和 `serde` 序列化。阻塞读取路径必须完整读完长度头和 payload 后才解码，避免粘包和半包问题；需要轮询时不能用无状态非阻塞 `read_message` 丢弃半包状态。

核心消息：

```text
Hello
StartSession
SessionReady
VideoReady
InputEvent
StopSession
Error
Heartbeat
Goodbye
```

连接流程：

```text
Client -> Host: Hello
Host -> Client: Hello
Client -> Host: StartSession
Host: 启动程序并建立捕获
Host -> Client: SessionReady
Host -> Client: VideoReady
Client <-> Host: 输入事件与状态消息
Client -> Host: StopSession
Host -> Client: Goodbye
```

当前 raw BGRA 阶段不需要 SDP offer/answer 或 ICE candidate。后续接入 WebRTC 时，控制通道再承载 SDP offer/answer 和 ICE candidate，信令过程隐藏在 `StartSession` 到 `VideoReady` 之间。

Service 与 Host Agent 的本机 IPC 消息模型已经独立放在协议 crate 中，覆盖 Service 向 Agent 发起会话启动、停止、锁屏通知，以及 Agent 向 Service 上报在线状态、会话已启动、会话已结束和错误。当前已补齐长度前缀 JSON frame 编解码，并在 Host 侧增加通用 Read/Write endpoint、最小 TCP loopback transport 和 Service 侧 request/ack coordinator，便于后续继续替换或扩展到命名管道、Unix domain socket 等通道；现阶段还没有定义连接生命周期、重连、心跳超时、消息投递重试策略，也尚未把会话命令接入真实 Agent 进程和 Host Agent runtime。

## 配置设计

宿主端配置示例：

```toml
listen = "0.0.0.0:7856"
program = "C:\\Program Files\\SomeApp\\app.exe"
args = []
work_dir = "C:\\Program Files\\SomeApp"

[video]
width = 1280
height = 720
fps = 30
codec = "raw_bgra"
bitrate_kbps = 4000

[capture]
mode = "window"
window_title_contains = "SomeApp"
startup_timeout_ms = 15000
```

当前配置模型仍要求填写 `bitrate_kbps`，但在 `codec = "raw_bgra"` 的当前主线下，该字段只是与后续编码路线共用的保留配置，不代表宿主端已经默认启用 H.264 编码传输。

客户端配置示例：

```toml
host = "192.168.10.25"
port = 7856
```

配置读取失败必须直接报错并退出，不做隐式猜测。路径不存在、工作目录不存在、端口被占用、帧率或码率非法，都应在启动时暴露。

## 错误处理

错误处理以“明确暴露，不静默重试”为原则。

宿主端需要区分：

- 配置错误。
- 监听端口失败。
- Windows 未登录、已锁屏或交互桌面不可用。
- 客户端协议错误。
- 程序启动失败。
- 程序启动后找不到窗口。
- 捕获初始化失败。
- 编码失败。
- 传输失败。
- 输入注入失败。

`CaptureFailed` 用于捕获能力本身失败，例如当前平台不支持捕获、Windows Graphics Capture 初始化失败、窗口句柄失效或捕获会话创建失败。`EncodingFailed` 仅用于后续 H.264 编码路线中的编码器初始化失败、首帧或后续帧编码失败，以及编码器产出非法帧。`TransportFailed` 用于控制通道、raw BGRA 帧通道、后续媒体传输、WebRTC/DataChannel 或编码后数据发送失败，不用于表达程序启动、窗口定位、捕获初始化或编码器本身错误。

客户端需要区分：

- 无法连接宿主端。
- 宿主端忙碌。
- 宿主机未登录。
- 宿主机已锁屏。
- 会话启动失败。
- 视频流中断。
- 输入发送失败。
- 协议版本不匹配。

错误信息面向最终用户时使用中文；日志保留必要的底层错误码和上下文。

## 资源生命周期

会话资源按连接生命周期管理：

```text
客户端连接建立
-> 创建 session
-> 启动程序
-> 创建捕获资源
-> 创建帧发送资源
-> 建立输入循环
-> 连接断开或用户停止
-> 停止捕获
-> 停止帧发送
-> 可配置是否关闭远程程序
-> 释放 session
```

断开连接后关闭由本次会话启动的程序。

## 测试与验证

Rust 代码修改完成后必须执行：

```text
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
```

除 `cargo fmt --all` 外，`cargo check`、`cargo clippy`、`cargo test`、`cargo build`、`cargo run`、`cargo doc` 等会写入 `target` 或运行 build script 的命令，执行时需要申请非沙箱权限，避免沙箱导致构建产物写入失败，例如 `拒绝访问 (os error 5)`。

验证场景：

- 宿主端配置错误时能给出明确错误。
- 客户端无法连接时能给出明确错误。
- 宿主机未登录时客户端能显示明确状态，不进入假会话。
- 宿主机锁屏时客户端能显示明确状态，并暂停或断开会话。
- 宿主机解锁后能恢复 Agent，客户端能重连。
- 客户端连接后能启动 Notepad 或其他轻量 Windows 程序。
- 客户端能看到远程画面。
- 鼠标移动、点击、滚轮可作用到远程程序。
- 键盘输入可作用到远程程序。
- 客户端断开后宿主端能释放资源并接受下一次连接。
- 目标窗口关闭后客户端能收到明确错误或会话结束事件。

## 实施顺序

建议按以下顺序推进：

- 当前 raw BGRA 主线已形成可用画面链路；客户端 `run` 已支持启动连接阶段的 `--retries` 和 `--retry-delay-ms` 有限重试；继续补齐资源释放、会话中断恢复、窗口关闭和错误上报，先让前台 Host Agent 形态稳定。
- 会话状态纯模型、平台事件映射边界和 Windows 会话通知注册/注销边界已完成；后续补齐消息循环、活动会话检测和桌面锁定事件分发，把登录、锁屏、解锁和注销事件转换为现有状态机事件。
- Service 与 Host Agent 的 IPC 消息模型、长度前缀 JSON frame 底座、Host 侧通用 endpoint、最小 TCP loopback transport 和 Service 侧 request/ack coordinator 已完成；后续继续处理连接建立、断开、超时、错误传播、权限边界，以及将会话命令接入真实 Agent 进程和 Host Agent runtime。
- Host Agent 运行核心已从 CLI 入口中剥离，网络会话、程序生命周期、捕获和输入编排已有可复用 runtime 边界；后续由 Service 拉起时复用该边界。
- `service` 命令已具备可测试的 ServiceManager 占位抽象；后续把该抽象接到真实 Windows Service 安装、卸载、启动、停止和开机自启，Service 只负责编排，不直接捕获和注入输入。
- 接入 Service 拉起 Host Agent 的流程，并把 Agent 在线状态、桌面可用状态和会话状态通过 IPC 回传给 Service。
- 在客户端展示宿主机未登录、已锁屏、Agent 不在线等状态，并实现可控重连。
- 在真机上验证重启后自动登录、锁屏、解锁、注销、断线重连和目标程序关闭等场景。
- raw BGRA 链路稳定后，再按性能需要接入低延迟编码和传输。
- 补打包、部署文档、基础测试和 CI 覆盖。

每一步都应形成可运行、可验证的闭环，避免同时堆叠捕获、编码、网络、输入和 UI 问题。

## 关键风险

- Windows 会话限制是结构性风险。系统必须接受宿主机可被接管，不能承诺不影响本地用户。
- 自动登录可以解决无人值守恢复，但会提高宿主机本地凭据暴露风险，需要配合专用低权限账号、物理访问限制和内网隔离。
- Service 不能替代用户交互桌面内的 Agent；捕获和输入注入必须落在正确的用户会话中。
- 锁屏和注销会天然中断捕获与输入，系统只能感知、停止和恢复，不能假装锁屏界面仍可控。
- raw BGRA 帧带宽较高，只适合作为当前内网优先闭环链路；高分辨率或高帧率场景需要后续 H.264/WebRTC 优化。
- 不同 Windows 版本和显卡驱动对窗口捕获、硬件编码支持存在差异，需要保留兜底路线。
- DPI、窗口缩放和多显示器会影响输入坐标映射，需要在协议里保留画面尺寸和捕获区域元数据。
- 输入法和组合键在跨系统场景下容易出现不一致，系统只保证基础键鼠事件。
- Rust 生态中 WebRTC、硬编、Windows 捕获之间的集成成本需要原型验证，必要时可用 GStreamer 降低后续媒体链路风险。
