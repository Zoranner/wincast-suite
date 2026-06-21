# WinCast Unity

WinCast Unity 是 Unity 内嵌远控后端的 runtime package。它负责在 Unity 进程内采集最终画面、把画面提交给 Rust native 库，并在 Unity 主线程轮询远端输入事件后分发给 UI 和业务控制层。

## 边界

- Unity package 负责最终帧采集、readback 调度、native bridge 调用和输入分发。
- Rust native 库负责网络、协议、编码、帧队列和输入事件队列。
- Host 进程负责读取配置、拉起一个 Unity Player、传入固定端口和前台监控进程生命周期。

## 运行结构

`WinCastUnityAgent` 是入口组件，挂到 Unity 场景中的一个 GameObject 上。它持有 `FinalFrameCapture`、`WinCastNativeBridge` 和 `RemoteInputGateway`，并在生命周期内启动、提交画面、轮询输入和关闭 native runtime。

`WinCastUnityAgent` 保留 Inspector 默认配置，同时只支持 Host 通过启动参数覆盖端口：`--wincast-port`。`--wincast-port value` 和 `--wincast-port=value` 两种形式都可解析。画面尺寸、目标帧率和码率均由 Unity 组件或项目配置决定，Host 不通过启动参数覆盖。Unity package 按单实例 Player 接入，native config 内部使用固定 runtime 标识，不暴露为 Inspector 或 Host 配置。

native runtime 默认监听 `0.0.0.0:<port>`，让 Linux client 可以直接连接 Unity Player 的远控端口。部署时需要在 Windows 防火墙中放通该端口，并按现场网络边界限制来源 IP 或网段。

`FinalFrameCapture` 使用 `WaitForEndOfFrame`、`ScreenCapture.CaptureScreenshotIntoRenderTexture` 和 `AsyncGPUReadback` 表达最终 Game View 捕获路径。这个路径用于覆盖以 Screen Space Overlay 为主的 UI，而不是抓取单个 Camera。

`RemoteInputGateway` 在 `Update` 中轮询 native 输入队列，并把事件分发给 `UiEventDispatcher` 和 `RemoteInputAdapter`。UI 事件通过 Unity `EventSystem` 处理，业务场景操作通过可替换的 adapter 扩展。

## 已完成开发项

当前 package 已补齐 Runtime assembly definition，Unity 项目可以按 `Zoranner.WinCast` assembly 引用 runtime 源码。

源码层已具备以下结构：

- native 输入队列轮询。
- pointer move/down/up/scroll 的基础 EventSystem 分发。
- Text 事件优先尝试写入当前选中的 Unity `InputField`。
- TextMeshPro `TMP_InputField` 按反射可选支持，不让 package 强依赖 TMP assembly。
- 未被 UI 消费的输入事件转交 `RemoteInputAdapter` 扩展点。
- Host 启动参数只识别 `--wincast-port`。

## 仍需验收项

- Unity Player 真机验证，包含最终 Game View 捕获路径、native DLL 加载和主线程轮询行为。
- 端到端客户端验收。
- 会话生命周期完善。
- 单实例稳定性和 UI 输入验收。

当前仓库不包含 Rust native DLL、Unity `.meta` 文件或 Unity Editor 内验证结果。接入时需要把 native DLL 放到 Unity 项目的插件目录，再进行 Player 运行验证。

第一阶段不通过 Host、启动参数或 native config 处理鉴权，也不要求 Unity package 配置 token。后续如需要鉴权能力，应按单实例会话生命周期单独设计授权边界。
