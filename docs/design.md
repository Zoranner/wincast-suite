# WinCast Suite 设计方案

## 项目定位

WinCast Suite 面向国产化操作系统访问 Windows 应用的内网远程应用工具。系统采用“专用 Windows 宿主机可被远程接管”的边界：Linux 客户端连接 Windows 宿主端后，由宿主端启动配置好的 Windows 程序，采集该程序窗口或桌面区域画面并传输给客户端，同时把客户端鼠标和键盘事件映射回 Windows 宿主机。客户端明确面向 Linux x86_64 与 Linux aarch64/ARM64 两类部署目标。

本项目使用 Rust 开发，优先保证链路清晰、部署简单、低延迟可验证。系统不追求替代 RDP/RemoteApp，也不承诺不影响宿主机本地使用。

## 使用边界

系统只支持内网或专网直连。客户端直接配置 Windows 宿主机 IP 地址和端口，宿主端直接配置监听端口、程序路径、启动参数、工作目录、画面参数等基础项。

Windows 宿主机需要满足以下前提：

- 已登录到一个可交互桌面。
- 桌面未锁屏。
- 推荐使用专用账号自动登录。
- 推荐接入物理显示器、虚拟显示器或 HDMI 虚拟头，以保证无人值守时仍有稳定分辨率。
- 被启动程序不应依赖 UAC 安全桌面交互。
- 宿主机本地若有人同时使用，会被远程输入和窗口焦点影响。

系统不处理 Windows 登录、解锁、锁屏界面、UAC 安全桌面、独立多用户远程会话，也不保证本地用户无感使用。

## 目标能力

- Linux x86_64 与 Linux aarch64/ARM64 客户端通过 IP 和端口连接 Windows 宿主端。
- Windows 宿主端根据配置启动一个指定程序。
- 宿主端定位目标程序窗口，捕获窗口画面；窗口捕获不稳定时允许退回桌面区域捕获。
- 宿主端以低延迟视频流方式传输画面。
- 客户端展示远程画面，并采集鼠标、键盘、滚轮事件。
- 宿主端接收输入事件并注入到 Windows 交互桌面。
- 连接断开后释放捕获、编码、进程和网络资源。
- 对启动失败、找不到窗口、捕获失败、连接断开等基础错误给出明确提示。

## 非目标能力

系统明确不做以下能力：

- 白名单、客户端证书、用户权限、审计日志。
- 空闲超时、崩溃重启、单实例或多实例策略。
- 敏感组合键过滤。
- 文件传输、剪贴板同步、音频、打印、USB 重定向。
- 管理端、服务发现、自动注册、中心调度。
- 公网访问、NAT 穿透、TURN 中继。
- 不影响宿主机本地用户的独立远程会话。
- 对 Windows 登录界面、锁屏界面、UAC 安全桌面的捕获和控制。

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
  -> 读取监听端口和程序配置
  -> 等待客户端连接
  -> 启动 Windows 程序
  -> 定位目标窗口
  -> 捕获窗口或桌面区域
  -> 发送画面帧
  -> 接收输入事件并注入
```

宿主端同一时间只处理一个客户端连接。若已有客户端连接，新的连接应返回“忙碌”错误并关闭。

## Rust 工程结构

推荐使用 Cargo workspace：

```text
crates/
  wincast-protocol/    # 控制消息、输入事件、配置模型、错误码
  wincast-host/        # Windows 宿主端主程序
  wincast-capture/     # Windows 画面捕获封装
  wincast-input/       # Windows 输入注入封装
  wincast-client/      # Linux x86_64 与 Linux aarch64/ARM64 客户端主程序
  wincast-render/      # 客户端 raw BGRA 渲染封装，第一阶段使用 SDL2
docs/
  design.md
```

各 crate 边界如下：

- `wincast-protocol` 不依赖平台 API，只定义可序列化的数据结构。
- `wincast-host` 负责配置读取、连接管理、进程生命周期和各模块编排。
- `wincast-capture` 只封装 Windows 捕获能力，不处理网络和进程启动。
- `wincast-input` 只封装 Windows 输入注入，不理解客户端 UI。
- `wincast-client` 负责连接宿主端、窗口生命周期和本地事件采集。
- `wincast-render` 负责帧缓冲和渲染输出；第一阶段使用 SDL2 在 Linux 上显示 raw BGRA 帧，后续接入 H.264 时再承担视频解码。

## 当前 CLI 骨架

当前 `wincast-host` 与 `wincast-client` 提供配置读取、配置校验和最小 TCP 控制通道握手，不代表媒体链路、捕获、渲染或输入链路已经完成。

宿主端 CLI：

```text
wincast-host --config wincast-host.toml
wincast-host --config wincast-host.toml validate
wincast-host --config wincast-host.toml run
```

客户端 CLI：

```text
wincast-client --config wincast-client.toml
wincast-client --config wincast-client.toml validate
wincast-client --config wincast-client.toml run
wincast-client targets
```

不带子命令时默认进入 `run`。宿主端 `run` 在配置校验通过后监听一次 TCP 连接，接受客户端 `Hello` 和 `StartSession` 控制消息，随后尝试启动配置程序、定位主窗口、通过 Windows Graphics Capture 初始化捕获会话、等待首帧 BGRA readback 缓冲，发送 `SessionReady`、`VideoReady` 和 raw BGRA 二进制帧；Linux 客户端 `run` 连接宿主端、发送 `Hello` 和 `StartSession`，创建 SDL2 窗口并渲染 raw BGRA 首帧，非 Linux 开发环境只执行协议校验路径，并把宿主端错误响应明确暴露出来。当前 `wincast-protocol` 已定义 raw BGRA 二进制帧、`VideoReady` 和后续可选 H.264 `EncodedVideoFrame` 线格式；当前主线优先打通 raw BGRA 帧链路，H.264/WebRTC 只作为后续性能优化项。`wincast-capture` 已接入 WGC 支持检测、窗口捕获目标创建、D3D11 设备、帧池、捕获会话启动、首帧等待、帧元数据读取、D3D11 纹理描述读取、尺寸变化后的帧池重建和可选 BGRA readback；`wincast-render` 已提供 SDL2 raw BGRA 窗口后端，但尚未实现长时间渲染循环、输入事件发送或输入注入。客户端 `targets` 必须明确列出 `x86_64-unknown-linux-gnu` 与 `aarch64-unknown-linux-gnu`，对应 Linux x86_64 与 Linux aarch64/ARM64。

## 宿主端设计

宿主端是 Windows 前台可执行程序，不设计为 Windows Service。前台运行可以直接处于当前交互桌面，避免 Session 0 与交互桌面隔离问题。

宿主端启动流程：

```text
读取配置
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

客户端第一阶段窗口后端固定为 SDL2，不引入 Slint 或 egui。这样可以降低银河麒麟 V10 等老系统上的 GPU、Vulkan、Wayland 和桌面环境依赖风险，优先通过 SDL2 streaming texture 直接显示 BGRA8888 帧。

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

控制通道使用长度前缀二进制消息和 `serde` 序列化，避免粘包和半包问题，同时保留调试日志输出。

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
codec = "h264"
bitrate_kbps = 4000

[capture]
mode = "window"
window_title_contains = "SomeApp"
startup_timeout_ms = 15000
```

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
- 客户端连接后能启动 Notepad 或其他轻量 Windows 程序。
- 客户端能看到远程画面。
- 鼠标移动、点击、滚轮可作用到远程程序。
- 键盘输入可作用到远程程序。
- 客户端断开后宿主端能释放资源并接受下一次连接。
- 目标窗口关闭后客户端能收到明确错误或会话结束事件。

## 实施顺序

建议按以下顺序推进：

- 建立 Rust workspace、配置模型、控制通道和本地回环协议测试。
- 实现 Windows 宿主端启动程序和窗口定位。
- 实现画面捕获原型，验证帧可取。
- 实现 raw BGRA 帧通道，先形成可读取首帧的端到端链路。
- 实现客户端窗口、raw BGRA 渲染和输入采集。
- 按性能需要接入低延迟编码和传输。
- 补错误处理、资源释放、基础测试和打包。

每一步都应形成可运行、可验证的闭环，避免同时堆叠捕获、编码、网络、输入和 UI 问题。

## 关键风险

- Windows 会话限制是结构性风险。系统必须接受宿主机可被接管，不能承诺不影响本地用户。
- raw BGRA 帧带宽较高，只适合作为内网优先闭环和原型路线；高分辨率或高帧率场景需要后续 H.264/WebRTC 优化。
- 不同 Windows 版本和显卡驱动对窗口捕获、硬件编码支持存在差异，需要保留兜底路线。
- DPI、窗口缩放和多显示器会影响输入坐标映射，需要在协议里保留画面尺寸和捕获区域元数据。
- 输入法和组合键在跨系统场景下容易出现不一致，系统只保证基础键鼠事件。
- Rust 生态中 WebRTC、硬编、Windows 捕获之间的集成成本需要原型验证，必要时可用 GStreamer 降低后续媒体链路风险。
