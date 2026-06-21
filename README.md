# WinCast Suite

WinCast Suite 用来在内网里从 Linux 客户端操作一台 Windows 机器上的指定程序。

Windows 端负责启动配置程序、捕获当前交互桌面整块屏幕、接收键鼠输入。Linux 端负责连接 Windows、显示画面、回传键鼠操作。

## 配置 Windows 端

配置文件放在：

```text
%APPDATA%\WinCast\host.toml
```

可以从 `examples/host.toml` 复制后修改：

```toml
listen = "0.0.0.0:7856"

[program]
path = 'C:\Program Files\SomeApp\app.exe'
args = []
work_dir = 'C:\Program Files\SomeApp'
startup_delay_ms = 3000
turn_off_monitor_after_launch = "disabled"

[video]
width = 1280
height = 720
fps = 30
codec = "h264"
bitrate_kbps = 4000
max_bitrate_kbps = 6000

[capture]
first_frame_timeout_ms = 5000
```

`program.path` 改成要打开的 Windows 程序路径。`program.startup_delay_ms` 表示程序启动后延迟多久开始整屏捕获。`program.turn_off_monitor_after_launch` 控制目标程序启动成功后的显示器处理策略，默认 `disabled`；当前只允许 `disabled` 和 `ddc_ci_dim`。`ddc_ci_dim` 通过 DDC/CI 尝试把显示器亮度调到最低，优先保持显示输出 active。`windows_power_message` 和 `ddc_ci_power_off` 会让显示器进入真正关屏状态，已知会破坏 DXGI Desktop Duplication 画面捕获，因此配置会被拒绝。DDC/CI 能力依赖显示器、线缆和显卡驱动支持，必须在目标 Windows 真机上烟测确认。`listen` 的端口要和客户端配置一致。

## 配置 Linux 端

配置文件放在：

```text
~/.config/wincast/client.toml
```

可以从 `examples/client.toml` 复制后修改：

```toml
host = "192.168.10.25"
port = 7856
```

`host` 改成 Windows 机器的 IP，`port` 和 Windows 端 `listen` 保持一致。

## 启动

Windows 端：

```powershell
wincast-host
```

Linux 端：

```bash
wincast-client
```

Linux 客户端启动后会立即打开全屏窗口，连接 Windows 端时显示加载进度，收到首帧后直接切换为 Windows 宿主机当前整块屏幕。鼠标和键盘操作会回传到 Windows 端。

如果会话期间 Windows 宿主端启动的目标程序自行退出，客户端会把本次会话视为正常结束并退出；如果关闭 Linux 客户端窗口，客户端会发送停止会话请求，Windows 端清理本次启动的程序树。

## Unity 内嵌后端

仓库已包含第一阶段 Unity 内嵌远控骨架：`crates/wincast-unity-native` 提供给 Unity 调用的 Rust native core FFI 边界，`unity/com.zoranner.wincast` 提供 Unity package 源码骨架。该后端目标是在 Unity 进程内采集最终 Game View 帧，并在 Unity 主线程消费远端输入，避免依赖 Windows 桌面抓屏和显示器电源状态。

当前这部分仍是工程骨架：已具备配置解析、runtime 状态、帧参数校验、最终帧采集结构、native bridge 声明、本地输入事件队列、Unity 输入分发骨架，以及 Rust native 侧最小 TCP 会话、协议输入入队和 H.264 编码帧发送链路；尚未完成 Unity Player 真机验证、端到端客户端验收、会话生命周期完善、单实例稳定性和 UI 输入验收。详细设计见 [docs/plans/2026-06-21-Unity内嵌远控后端设计.md](docs/plans/2026-06-21-Unity内嵌远控后端设计.md)。

Host 侧可通过 `mode = "unity_embedded"` 选择 Unity 内嵌后端。该模式需要额外提供 `[unity]` 配置；Host 读取配置后只拉起一个 Unity 进程，并以 `--wincast-port <port>` 传入固定端口，随后在前台监控该 Unity 进程。Unity 进程的分辨率、FPS、码率和鉴权策略由 Unity package 或具体 Unity 项目自身配置，Host 不通过启动参数覆盖。

```toml
mode = "unity_embedded"

[unity]
executable = 'C:\Program Files\SomeUnityApp\UnityApp.exe'
work_dir = 'C:\Program Files\SomeUnityApp'
port = 7900
```

## 部署与烟测

部署前先按 [docs/deployment-prep.md](docs/deployment-prep.md) 准备两端产物、配置、依赖和防火墙。完成部署准备后，再按 [docs/smoke-test.md](docs/smoke-test.md) 执行 Windows host、Linux x86_64 client 和 Linux aarch64/ARM64 client 真机烟测。

## 使用限制

- Windows 端需要处于已登录桌面。
- 当前只支持一台客户端连接。
- 当前只支持单显示器。
- 当前只捕获整块屏幕，不支持窗口捕获或多屏选择。
- 锁屏、注销或网络中断时，本次连接会断开，客户端会尝试重新连接。
