# WinCast Suite 设计方案

## 项目定位

WinCast Suite 面向内网或专网环境，用于让 Linux 客户端远程使用一台 Windows 宿主机上的指定程序。Windows 宿主端负责按配置启动程序，等待固定延迟后捕获当前交互桌面整块屏幕，将 H.264 编码视频流传给 Linux 客户端，并把客户端鼠标键盘事件注入回 Windows 交互桌面。

本项目不是通用远控产品，也不追求替代 RDP、RemoteApp 或 RustDesk。当前稳定版只面向受控内网、固定宿主机、单客户端、单显示器、最高 1920x1080 的远程应用操控场景。目标程序可以是全屏 Unity 程序、全屏仿真软件或普通桌面程序，但系统只捕获整块屏幕，不再维护窗口捕获、窗口标题定位和捕获模式选择。

系统采用 Rust 开发。捕获、编码、解码、渲染、输入等成熟领域能力优先使用可靠第三方库或系统 API 封装，不从零实现底层媒体栈。

## 使用边界

系统只支持内网或专网直连。客户端直接配置 Windows 宿主机 IP 和端口，宿主端读取本地配置并监听连接。不提供公网访问、NAT 穿透、TURN 中继、服务发现、账号体系、客户端证书、权限分级或审计日志。

当前稳定目标限定如下：

- Windows 宿主机只有一个可用显示器。
- 宿主显示器最高按 1920x1080 设计，不主动降分辨率。
- 宿主程序由配置启动，启动后按固定延迟进入捕获。
- 捕获范围固定为当前交互桌面整屏，屏幕上出现的任务栏、弹窗和其他窗口都会被传输。
- 客户端覆盖 Linux x86_64 与 Linux aarch64/ARM64。
- 同一时间只允许一个客户端控制宿主端。
- 宿主端可以被远程输入影响，不承诺不影响本地用户。
- 宿主端推荐使用专用账号自动登录，保证重启后可进入交互桌面。
- 锁屏、注销或交互桌面不可用时，系统只能感知、断开或恢复，不做远程凭据输入。

安全边界按受控内网工具处理，只保留必要工程底线：Host 监听显式配置的地址和端口；配置文件不保存 Windows 登录密码；错误状态明确暴露，不伪装在线。

## 目标能力

- Linux 客户端通过 IP 和端口连接 Windows 宿主端。
- Windows 宿主端按配置启动指定程序。
- 宿主端可按 `program.turn_off_monitor_after_launch` 在目标程序启动成功后尝试调暗显示器，默认不处理显示器。
- 宿主端等待 `program.startup_delay_ms` 后开始整屏捕获，等待期间客户端停止、断开、会话不可用或目标程序退出都必须中止会话。
- Windows 10 1809 / Build 17763 宿主端使用 DXGI Desktop Duplication 捕获当前交互桌面整屏。
- 视频传输使用低延迟 H.264 编码流，不设计 raw BGRA 网络传输链路。
- 客户端解码视频流并显示远程画面。
- 客户端采集鼠标移动、按下、释放、滚轮和键盘事件。
- 输入重点覆盖仿真软件常见操作：拖动镜头旋转、拖动平移、滚轮缩放和普通键盘输入。
- 客户端关闭或连接断开后，宿主端释放捕获、编码、输入、网络和本次会话启动的程序资源。
- Windows 宿主端可执行程序负责常驻监听、启动配置程序、整屏捕获和输入注入。
- 登录、锁屏、解锁和注销状态需要被感知，并转换为客户端可理解的状态或错误。

## 非目标能力

系统明确不做以下能力：

- 公网远控、NAT 穿透、TURN 中继、UDP 媒体通道、WebRTC。
- 账号体系、客户端证书、权限分级、审计日志。
- 多客户端并发访问同一宿主端。
- 多显示器枚举、跨屏坐标、窗口跨屏迁移、显示器动态插拔。
- 窗口捕获、窗口标题匹配、窗口句柄跟踪、按窗口裁剪画面。
- 虚拟显示器驱动和无物理显示器兜底。
- 文件传输、剪贴板同步、音频、打印、USB 重定向。
- 敏感组合键过滤、完整远程输入法、手柄输入。
- 不影响宿主机本地用户的独立远程会话。
- 远程输入 Windows 账号密码来登录或解锁。
- Windows 登录界面、锁屏界面、UAC 安全桌面的捕获和控制。
- Credential Provider、自定义登录凭据入口或替代 Windows 登录界面。

其中远程登录/解锁、文件传输、剪贴板同步、多客户端并发、多屏、公网访问、UDP 媒体通道和 WebRTC 是永久非目标，不作为后续路线图。

## 总体架构

系统由 Windows 宿主端和 Linux 客户端组成。两端共享协议 crate，控制消息、输入事件、错误码和配置模型保持一致。

```text
Linux Client
  -> 读取 host/port
  -> 建立内网连接
  -> 接收 H.264 编码视频帧
  -> 解码并显示远程画面
  -> 采集鼠标键盘事件
  -> 发送输入事件

Windows Host
  -> 读取本地配置并常驻监听
  -> 启动目标程序
  -> 可取消等待 startup_delay_ms
  -> 捕获当前交互桌面整屏
  -> H.264 低延迟编码
  -> 发送编码视频帧
  -> 接收并注入输入事件
  -> 会话结束时清理本次启动的程序
```

Windows Host 必须运行在实际用户交互桌面中，不通过 Session 0 服务进程转接捕获和输入。

## Rust 工程结构

当前 Cargo workspace 按能力拆分：

```text
crates/
  wincast-protocol/    # 控制消息、输入事件、配置模型、错误码与视频帧
  wincast-host/        # Windows 宿主端可执行程序、监听入口、会话生命周期
  wincast-capture/     # Windows DXGI Desktop Duplication 整屏捕获封装
  wincast-media/       # 视频编码、解码、码率/FPS 控制和媒体帧模型
  wincast-input/       # Windows 输入注入封装
  wincast-client/      # Linux x86_64 与 Linux aarch64/ARM64 客户端主程序
  wincast-render/      # Linux 客户端窗口、解码后画面呈现和输入采集
docs/
  design.md
```

工程边界要求：

- `wincast-protocol` 只描述跨进程和跨平台契约，不依赖捕获、媒体、输入实现。
- `wincast-capture` 只暴露整屏捕获目标和捕获帧模型，不暴露窗口概念。
- `wincast-host` 组合程序启动、会话门禁、捕获、编码、输入和清理流程，不直接实现底层媒体算法。
- `wincast-media` 负责编码/解码和编码帧边界，避免媒体逻辑散落在 host/client 入口。
- `wincast-input` 负责 Windows 输入注入和坐标映射边界。
- `wincast-client` 与 `wincast-render` 分离连接协议、解码和 SDL2 窗口事件处理。

## 第三方库与系统能力策略

项目不应自己实现成熟媒体和系统底层能力。当前稳定实现固定为 OpenH264 软件 H.264 编解码、SDL2 客户端窗口和 Windows 系统捕获/输入 API。

- 捕获：Windows 宿主端使用 DXGI Desktop Duplication，覆盖 Windows 10 1809 / Build 17763。
- 编码：当前使用 OpenH264 软件 H.264 编码，避免手写 H.264 编码器。
- 解码：Linux 客户端当前使用 OpenH264 软件 H.264 解码，并必须保证 x86_64 与 aarch64/ARM64 可构建、可部署。
- 渲染与输入采集：客户端窗口继续优先 SDL2，除非目标系统证明 SDL2 不可用；不要引入复杂 GUI 框架只为显示视频画面。
- 输入注入：Windows 侧使用 `SendInput` 或成熟封装，输入协议只覆盖当前稳定目标内的基础键鼠与拖拽操作。

RustDesk 参考项目的价值在于捕获抽象、编码/QoS 和输入事件完整性，不在于照搬其公网远控、账号、文件、剪贴板、音频、打印、多端 UI 或插件体系。

## 捕获设计

稳定版捕获目标固定为当前交互桌面整屏：

```text
CaptureTarget::Screen
```

宿主端收到 `StartSession` 后先完成桌面会话门禁检查，再启动配置程序，然后等待 `program.startup_delay_ms`。延迟结束后创建 DXGI Desktop Duplication 会话并等待首帧。`capture.first_frame_timeout_ms` 只用于捕获会话启动后的首帧保护，避免后端异常时无限等待。

捕获模块需要处理：

- 枚举主显示输出失败。
- Direct3D 设备初始化失败。
- DXGI Desktop Duplication 创建失败。
- 捕获启动后首帧超时。
- 捕获过程中暂时无新帧。
- 捕获资源失效后的明确错误上报和资源释放。
- 整屏捕获下的输入坐标映射。

当前不处理多屏、虚拟显示器、窗口裁剪、UAC 安全桌面、锁屏界面和登录界面捕获。

## 视频编码与传输

正式传输链路只保留编码视频流。设计上删除 raw BGRA 作为网络协议、配置 codec 和烟测主线的概念。

```text
Capture Frame
  -> H.264 low-latency encoder
  -> EncodedVideoFrame
  -> framed stream over internal network
  -> Linux decoder
  -> renderer
```

默认目标：

- codec：H.264。
- resolution：编码输出尺寸由 `video.width` 与 `video.height` 决定，最高 1920x1080；整屏捕获画面与配置尺寸不一致时，宿主端在编码前缩放到配置尺寸。
- fps：默认 30。
- latency：优先低延迟，允许为低延迟牺牲压缩率。
- bitrate：配置目标码率和上限，当前不做复杂自适应码率。

H.265、AV1、硬件解码、60 FPS、更复杂的自适应码率、UDP 媒体通道和 WebRTC 不属于当前需求范围。

协议层表达编码帧，而不是 raw 像素帧。编码帧至少包含：

- codec。
- frame kind。
- width 与 height。
- sequence。
- timestamp。
- payload length。
- keyframe 标记。

慢客户端不应让编码帧无限积压。服务端和客户端需要有丢弃旧帧、请求关键帧、限制 FPS 或降低码率的机制。参考 RustDesk 的质量状态和 FPS 反馈思路，但 WinCast 不需要做复杂画质 UI。

## 输入映射

输入协议需要服务仿真软件的高频拖拽场景，而不只是普通点击。

基础输入事件包括：

- 鼠标移动。
- 鼠标按下和释放。
- 鼠标滚轮。
- 键盘按下和释放。
- 修饰键状态。

客户端窗口打开后应关闭本机文本输入/IME 组合状态，只采集按键按下和释放事件。远程画面中的输入法状态以 Windows 宿主端为准，客户端本机输入法不能参与键盘事件判断。

设计上保留相对移动或拖拽序列扩展点：

```text
MouseMoveAbsolute { x, y }
MouseMoveDelta { dx, dy }
MouseButton { button, state }
MouseWheel { delta_x, delta_y }
Keyboard { key, state, modifiers }
```

整屏捕获时，绝对坐标直接从客户端显示区域映射到远程屏幕坐标。若目标仿真软件依赖连续拖动来旋转或平移镜头，客户端必须保证按下、移动序列、释放顺序稳定；连接中断、窗口失焦或客户端退出时，宿主端需要释放仍处于按下状态的按键，避免粘键或粘鼠标按钮。

验收重点：

- 左键长按拖动旋转。
- 右键或中键长按拖动平移。
- 快速来回拖动不跳点。
- 滚轮缩放连续可用。
- 释放事件不丢失。
- 客户端窗口退出后宿主端没有残留按下状态。

当前不实现完整远程输入法、手柄输入、敏感组合键策略或游戏级 raw input 捕获。

## 协议设计

控制消息使用长度前缀二进制消息和 `serde` 序列化。阻塞读取路径必须完整读完长度头和 payload 后才解码，避免粘包和半包问题。

核心控制消息：

```text
Hello
StartSession
SessionReady
EncodedVideoFrame
InputEvent
RequestKeyFrame
QualityFeedback
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
Host: 检查桌面会话状态
Host: 启动配置程序
Host: 可取消等待 startup_delay_ms
Host: 初始化整屏捕获
Host: 初始化编码器
Host -> Client: SessionReady
Host -> Client: EncodedVideoFrame...
Client -> Host: InputEvent...
Client -> Host: StopSession
Host -> Client: Goodbye
```

控制消息、输入事件和编码视频帧共用当前 TCP 连接。当前设计不要求 SDP offer/answer、ICE candidate、UDP 媒体通道或 WebRTC 信令。

## 配置设计

宿主端配置示例：

```toml
listen = "0.0.0.0:7856"

[program]
path = 'C:\Program Files\SomeApp\app.exe'
args = []
work_dir = 'C:\Program Files\SomeApp'
startup_delay_ms = 3000
turn_off_monitor_after_launch = "disabled"

[video]
width = 1920
height = 1080
fps = 30
codec = "h264"
bitrate_kbps = 8000
max_bitrate_kbps = 12000

[capture]
first_frame_timeout_ms = 5000
```

Unity 内嵌后端配置在稳定 desktop_dxgi 配置基础上增加 `mode` 和 `[unity]`：

```toml
mode = "unity_embedded"

[unity]
executable = 'C:\Program Files\SomeUnityApp\UnityApp.exe'
work_dir = 'C:\Program Files\SomeUnityApp'
port = 7900
```

客户端配置示例：

```toml
host = "192.168.10.25"
port = 7856
```

配置原则：

- `program.path`、`program.args` 和 `program.work_dir` 描述本次会话启动的 Windows 程序。
- `program.startup_delay_ms` 是程序启动后的固定等待时间，允许为 0。
- `program.turn_off_monitor_after_launch` 控制目标程序启动成功后的显示器处理，默认 `disabled`，不处理显示器。
- `program.turn_off_monitor_after_launch = "disabled"` 表示保持默认显示器状态。
- `program.turn_off_monitor_after_launch = "ddc_ci_dim"` 表示通过 DDC/CI 尝试把显示器亮度调到最低，优先保持显示输出 active，依赖显示器、线缆和显卡驱动支持。
- `program.turn_off_monitor_after_launch = "windows_power_message"` 和 `"ddc_ci_power_off"` 表示真正关闭显示器，会破坏 DXGI Desktop Duplication 画面捕获，配置必须拒绝。
- `video.codec` 当前只支持 `h264`。
- `video.width` 与 `video.height` 是 `desktop_dxgi` 后端的 H.264 编码输出尺寸，宿主端会把整屏捕获画面缩放到该尺寸后再编码。
- `capture.first_frame_timeout_ms` 是整屏捕获启动后的首帧保护超时，必须大于 0。
- `mode` 默认是 `desktop_dxgi`，保持现有桌面 DXGI 后端配置兼容；`unity_embedded` 需要提供 `[unity]`。
- `[unity].port` 描述单 Unity 进程内嵌远控监听端口。
- `unity_embedded` 后端读取配置后只拉起一个 Unity 进程，并只传入 `--wincast-port <port>`；分辨率、FPS、码率和鉴权策略由 Unity package 或具体 Unity 项目自身配置，Host 不通过启动参数覆盖。
- 配置读取失败必须直接报错，不做隐式猜测。
- Windows 路径在 TOML 中推荐用单引号，避免 `\` 被当作转义符。

## 宿主端运行设计

运行 `wincast-host` 后，宿主端直接读取默认配置并进入持续监听。该进程负责：

- 运行在用户交互桌面。
- 读取默认宿主端配置。
- 监听客户端连接。
- 在会话开始时启动配置程序。
- 按配置在目标程序启动成功后处理显示器调暗策略。
- 等待配置的启动延迟，等待期间继续响应客户端停止、断开、桌面会话不可用和目标程序退出。
- 初始化整屏捕获和 H.264 编码器。
- 接受客户端连接并发送编码视频帧。
- 接收输入事件并注入到 Windows 交互桌面。
- 客户端断开、停止会话或目标程序退出时清理本次会话资源；Windows 侧通过 Job Object 管理本次启动的程序树。

## 登录与锁屏处理

登录和锁屏处理以“感知状态、断开或恢复、不远程提交 Windows 凭据”为原则。

系统运行时需要区分：

- 未登录：宿主端进程无法进入交互桌面运行；客户端连接时应返回宿主机未登录。
- 已登录且未锁屏：宿主端进程保持监听，允许建立远程会话。
- 已锁屏：停止新会话；已有会话断开或暂停，客户端显示宿主机已锁定。
- 解锁后：宿主端重新确认交互桌面可用，客户端可重新连接。
- 注销或切换用户：释放捕获、编码、输入和会话资源。

系统不做 Credential Provider，不采集、不保存、不转发 Windows 登录密码。自动登录属于部署策略，不是远程解锁能力。

## 客户端设计

客户端只面向 Linux x86_64 与 Linux aarch64/ARM64。客户端职责是连接、解码、显示和输入采集，不承担业务管理能力。

客户端运行流程：

```text
读取配置
-> 连接 Host
-> Hello 握手
-> StartSession
-> 等待 SessionReady
-> 初始化 H.264 解码器
-> 创建 SDL2 窗口
-> 解码并显示视频帧
-> 采集输入并发送
-> 退出时发送 StopSession
```

客户端必须在目标 Linux 真机上验证 SDL2、解码库、窗口创建、渲染和输入采集。Windows 开发机上的编译和协议测试不能替代 Linux x86_64 或 ARM64 真机验证。

## 错误处理

错误处理以“明确暴露，不伪装成功”为原则。

宿主端需要区分：

- 配置错误。
- 监听端口失败。
- Windows 未登录。
- Windows 已锁屏。
- 客户端协议错误。
- 程序启动失败。
- 程序启动后在延迟期间或会话期间退出。
- 桌面输出枚举失败。
- 整屏捕获初始化失败。
- 捕获首帧超时。
- 编码器初始化失败。
- 编码帧生成失败。
- 传输失败。
- 输入注入失败。

客户端需要区分：

- 无法连接宿主端。
- 宿主端忙碌。
- 宿主机未登录。
- 宿主机已锁屏。
- 会话启动失败。
- 视频解码失败。
- 视频流中断。
- 输入发送失败。
- 协议版本不匹配。

错误信息面向最终用户时使用中文；日志保留必要底层错误码和上下文。

## 资源生命周期

会话资源按连接生命周期管理：

```text
客户端连接建立
-> 创建 session
-> 启动程序
-> 可取消等待 startup_delay_ms
-> 创建整屏捕获资源
-> 创建编码器
-> 建立视频发送循环
-> 建立输入接收循环
-> 连接断开或用户停止
-> 释放按下状态
-> 停止输入循环
-> 停止视频发送
-> 停止编码器
-> 停止捕获
-> 关闭本次会话启动的程序
-> 释放 session
```

任何异常路径都必须落到资源释放流程，尤其是鼠标按键释放、编码器释放、捕获资源释放和本次启动程序树清理。

## 测试与验证

Rust 代码修改完成后必须执行：

```text
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
```

同时应执行 workspace 测试：

```text
cargo test --workspace --all-targets --all-features
```

设计验收场景：

- 宿主端配置错误时能给出明确错误。
- 客户端无法连接时能给出明确错误。
- 宿主机未登录或锁屏时客户端能显示明确状态。
- Host 收到会话请求后能启动配置程序。
- `program.turn_off_monitor_after_launch = "disabled"` 时，Host 在目标程序启动成功后继续保持默认显示器状态。
- `program.turn_off_monitor_after_launch = "windows_power_message"` 时，Host 拒绝配置，因为真正关闭显示器会破坏 DXGI Desktop Duplication 画面捕获。
- `program.turn_off_monitor_after_launch = "ddc_ci_power_off"` 时，Host 拒绝配置，因为 DDC/CI 硬件关屏会破坏 DXGI Desktop Duplication 画面捕获。
- `program.turn_off_monitor_after_launch = "ddc_ci_dim"` 时，Host 通过 DDC/CI 尝试把显示器亮度调到最低；需要目标 Windows 真机确认显示器支持、亮度恢复策略、DXGI 采集状态和失败错误暴露。
- Host 等待 `program.startup_delay_ms` 后开始整屏捕获。
- 1080p、30 FPS、H.264 链路能持续运行。
- 左键拖动旋转、右键或中键拖动平移、滚轮缩放可用。
- 快速拖动不明显跳点，释放事件不丢失。
- 客户端退出后宿主端释放资源并接受下一次连接。
- 客户端退出或会话结束后，本次启动的宿主端程序被清理。
- 目标程序关闭后客户端收到明确错误或会话结束事件。
- Linux x86_64 和 Linux ARM64 真机完成客户端解码、渲染和输入采集验证。

## 实施顺序

当前稳定版按以下顺序推进：

- 收敛协议和配置口径，删除旧窗口捕获、捕获模式和窗口标题定位字段。
- 将捕获公共 API 固定为整屏目标。
- 将 Windows 捕获实现固定到 DXGI Desktop Duplication。
- 保持 H.264 编码、传输、Linux 解码和渲染闭环。
- 补拖拽输入专项，覆盖连续移动、释放收尾和异常断开释放。
- 完善登录、锁屏、解锁和注销状态感知。
- 在 Windows 宿主机和 Linux x86_64/ARM64 真机执行稳定版烟测。
- 补打包和部署文档。

每一步都应形成可运行、可验证的闭环，避免同时堆叠捕获、编码、传输、输入和锁屏恢复问题。

## 关键风险

- 整屏捕获会暴露目标程序之外的桌面内容，部署时必须使用专用宿主账号和受控桌面环境。
- 真正关闭显示器会破坏 DXGI Desktop Duplication 采集前提，`windows_power_message` 和 `ddc_ci_power_off` 必须被拒绝。
- DDC/CI 调暗依赖显示器、线缆、显卡驱动与 VCP 能力支持，必须在目标 Windows 真机上烟测确认，不能承诺一定降低物理可见性。
- DXGI Desktop Duplication 依赖已登录交互桌面，锁屏、注销和安全桌面不可控。
- 1080p 原分辨率下 raw 像素传输不可接受，因此 H.264 编码链路是核心前提。
- 第三方编码/解码库在 Windows、Linux x86_64 和 Linux ARM64 上的构建与部署成本需要持续验证。
- 捕获和输入必须发生在正确用户会话中。
- DPI、显示缩放和客户端窗口缩放会影响坐标映射，需要以真实仿真软件烟测确认。
- 输入法和复杂组合键不作为稳定目标，系统只保证基础键鼠和拖拽操控。
