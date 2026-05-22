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

`program.path` 改成要打开的 Windows 程序路径。`program.startup_delay_ms` 表示程序启动后延迟多久开始整屏捕获。`listen` 的端口要和客户端配置一致。

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

## 部署与烟测

部署前先按 [docs/deployment-prep.md](docs/deployment-prep.md) 准备两端产物、配置、依赖和防火墙。完成部署准备后，再按 [docs/smoke-test.md](docs/smoke-test.md) 执行 Windows host、Linux x86_64 client 和 Linux aarch64/ARM64 client 真机烟测。

## 使用限制

- Windows 端需要处于已登录桌面。
- 当前只支持一台客户端连接。
- 当前只支持单显示器。
- 当前只捕获整块屏幕，不支持窗口捕获或多屏选择。
- 锁屏、注销或网络中断时，本次连接会断开，客户端会尝试重新连接。
