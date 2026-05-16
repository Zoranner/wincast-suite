# WinCast Suite

WinCast Suite 用来在内网里从 Linux 客户端操作一台 Windows 机器上的指定程序。

Windows 端负责启动程序、捕获画面、接收键鼠输入。Linux 端负责连接 Windows、显示画面、回传键鼠操作。

## 配置 Windows 端

配置文件放在：

```text
%APPDATA%\WinCast\host.toml
```

可以从 `examples/host.toml` 复制后修改：

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
max_bitrate_kbps = 6000

[capture]
mode = "auto"
window_title_contains = "SomeApp"
startup_timeout_ms = 15000
```

`program` 改成要打开的 Windows 程序路径。`listen` 的端口要和客户端配置一致。

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

客户端启动后会连接 Windows 端，打开窗口并显示目标程序画面。鼠标和键盘操作会回传到 Windows 端。

## 使用限制

- Windows 端需要处于已登录桌面。
- 当前只支持一台客户端连接。
- 当前只支持单显示器。
- 锁屏、注销或网络中断时，本次连接会断开，客户端会尝试重新连接。

更多部署步骤见 [docs/smoke-test.md](docs/smoke-test.md)。
