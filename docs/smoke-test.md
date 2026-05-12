# 稳定版真机烟测清单

本清单只覆盖当前稳定版默认链路：Windows 前台 Host Agent、窗口捕获、raw BGRA 传输、Linux SDL2 客户端显示与基础键鼠输入。不要把通过项外推为 Service、锁屏恢复、desktop 捕获、H.264/WebRTC 或 ARM64 真机窗口运行已经完成。

## 准备环境

- Windows host 与 Linux client 位于同一可达网络，Windows 防火墙已放通 Host 监听端口。
- Windows host 使用可交互桌面登录，待捕获应用能以前台程序启动，并且目标窗口标题包含稳定可匹配文本。
- Linux client 已安装 SDL2 运行/开发依赖；ARM64 目标机需要在真实 aarch64/ARM64 Linux 设备上执行同一流程。
- 两端使用同一版本产物，Host 使用 `wincast-host`，Client 使用 `wincast-client`。

## 配置

配置文件以仓库内 `examples/` 目录为准，烟测时复制示例文件到默认用户配置目录后按现场环境调整，避免在清单里维护第二份 TOML。日常运行不需要每次传 `--config`；`--config` 仅用于临时调试或一次性验证时覆盖默认路径。

- Windows host 复制 `examples/wincast-host.toml` 为 `%APPDATA%\WinCast\wincast-host.toml`。
- Linux client 复制 `examples/wincast-client.toml` 为 `${XDG_CONFIG_HOME:-$HOME/.config}/wincast/wincast-client.toml`。`XDG_CONFIG_HOME` 必须是非空绝对路径；未设置、为空或为相对路径时使用 `$HOME/.config/wincast/wincast-client.toml`。

Windows host 保持窗口捕获和 raw BGRA，重点核对以下字段：

- `listen`：Host 监听地址，默认端口需与 Client 的 `port` 一致。
- `program`、`args`、`work_dir`：待捕获应用的启动命令和工作目录。
- `video.codec`：稳定版烟测使用 `raw_bgra`。
- `capture.mode`：稳定版烟测使用 `window`，不要改成 `desktop`。
- `capture.window_title_contains`：必须匹配目标应用窗口标题。
- `capture.startup_timeout_ms`：目标应用启动和窗口出现的等待时间。

Linux client 指向 Windows host，重点核对以下字段：

- `host`：Windows host 的可达 IP 或主机名。
- `port`：必须与 Host 配置中的 `listen` 端口一致。

## 执行步骤

- 在 Windows host 上执行 `wincast-host validate`，确认配置有效；如果提示 `desktop 捕获尚未实现`，改回 `capture.mode = "window"`。
- 在 Windows host 上执行 `wincast-host run`，确认控制通道进入持续监听。
- 在 Linux client 上执行 `wincast-client validate`，确认目标地址正确。
- 在 Linux client 上执行 `wincast-client run --retries 3 --retry-delay-ms 1000`，确认能建立连接并打开 SDL2 窗口。
- 观察客户端窗口，确认能看到 Windows 目标应用窗口画面，且窗口移动或目标应用内容变化后客户端画面随之更新。
- 在客户端窗口内移动鼠标、点击、滚轮滚动，并在目标应用可输入区域敲入普通字符，确认 Windows 目标应用收到鼠标和键盘输入。
- 关闭 Linux 客户端 SDL2 窗口，确认客户端退出时发送 `StopSession`，Windows host 结束当前会话并清理捕获/输入链路。
- 不重启 Windows host，再次启动 Linux client 连接同一 host，确认 host 能接受下一次连接并重新看到画面。

## 通过标准

- Host 以前台 `run` 模式启动并保持监听，窗口捕获配置能进入 raw BGRA 会话。
- Linux client 能连接 host，打开窗口并持续显示目标应用画面。
- 鼠标移动、点击、滚轮和普通键盘输入能从客户端回传到 Windows 目标应用。
- 客户端关闭窗口后会话结束，host 没有卡死在旧会话，能接受下一次客户端连接。

## 当前不支持项

- Windows Service 安装、启动、停止、状态查询和 Service 拉起 Host Agent 仍未实现，烟测必须使用前台 `run` 模式。
- 锁屏、解锁后的自动恢复或自动重连编排仍未实现。
- `capture.mode = "desktop"` 仍不是稳定版可用路径。
- H.264 编码传输和 WebRTC 传输仍未接入；稳定版烟测只验证 raw BGRA。
- ARM64 Linux 客户端虽然有交叉编译边界，但 SDL2 链接、窗口创建、渲染和输入回传仍必须在目标 ARM64 真机上验证。
