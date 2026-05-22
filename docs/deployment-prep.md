# 部署准备说明

## 适用范围

本文档只覆盖真机烟测前的部署准备，不替代 [稳定版真机烟测清单](smoke-test.md)。真机烟测必须在本准备项完成后执行，不能用 Windows 开发机上的 workspace 测试、交叉编译检查或协议单元测试代替。

## 产物准备

Windows 宿主端需要 `wincast-host` 可执行程序。Linux 客户端需要对应目标架构的 `wincast-client` 可执行程序。两端应来自同一代码版本，避免协议结构、配置字段或 H.264 帧边界不一致。

仓库发布流程由 `.github/workflows/release.yml` 管理。推送 `v*` 版本标签后，CI 会先执行工程门禁，再构建并上传以下 Release 资产：

- `wincast-host-x86_64-pc-windows-msvc.zip`：Windows 宿主端包，包含 `wincast-host.exe`、宿主端示例配置和部署文档。
- `wincast-client-x86_64-unknown-linux-gnu.tar.gz`：Linux x86_64 客户端包，包含 `wincast-client`、客户端示例配置和部署文档。
- `wincast-client-aarch64-unknown-linux-gnu.tar.gz`：Linux aarch64/ARM64 客户端包，包含 `wincast-client`、客户端示例配置和部署文档。
- 每个压缩包对应一个 `.sha256` 校验文件。

Linux 客户端包在 Debian buster 用户态容器内构建，并在 CI 中检查产物不得依赖高于 `GLIBC_2.28` 的符号，避免在旧版 Linux 目标机上出现 `GLIBC_x.xx not found`。

本地开发阶段可以使用 Cargo 产物；交付阶段应固定产物目录和版本标识。复制产物时同时记录：

- Git 提交号。
- 构建目标平台。
- 构建命令。
- 是否已经完成本地 `cargo fmt`、`cargo test` 和 `cargo clippy`。
- 是否已经完成对应 Linux 真机运行验证。

## Windows 宿主端准备

Windows host 必须运行在已登录的交互桌面中，不作为 Session 0 后台服务运行。部署前检查：

- 宿主机只有一个需要参与捕获的显示器。
- Windows 10 1809 / Build 17763 使用 DXGI Desktop Duplication 捕获当前整块屏幕。
- 待启动程序能由配置中的 `program.path`、`program.args` 和 `program.work_dir` 启动。
- Windows 防火墙允许 Host 配置中的监听端口入站访问。
- `%APPDATA%\WinCast\host.toml` 已从 `examples/host.toml` 复制并按现场环境修改。

Host 配置重点检查：

- `listen` 与客户端 `port` 一致。
- `video.codec` 固定为 `h264`。
- `video.width` 与 `video.height` 不超过稳定版目标上限。
- `program.startup_delay_ms` 足够覆盖 Unity 或目标程序进入全屏画面的时间。
- `capture.first_frame_timeout_ms` 大于 0，用于避免捕获后端异常时无限等待首帧。

## Linux 客户端准备

Linux client 源码构建需要安装 C/C++ 编译工具链、`pkg-config`、`cmake` 和 SDL2 bundled static 构建所需的系统图形/音频开发库。OpenH264 和 bundled SDL2 都会在构建时编译 C/C++ 源码，不能只检查 Rust 工具链。

Debian/Ubuntu 类系统可按开发说明安装：

```bash
sudo apt-get update
sudo apt-get install -y build-essential cmake pkg-config \
  libasound2-dev libdbus-1-dev libgl1-mesa-dev libibus-1.0-dev \
  libpulse-dev libudev-dev libwayland-dev libx11-dev libxcursor-dev \
  libxext-dev libxi-dev libxinerama-dev libxkbcommon-dev libxrandr-dev \
  libxrender-dev libxss-dev libxtst-dev
```

银河麒麟 V10 等非 `apt` 系统应使用对应包管理器安装同类依赖，再执行相同 Cargo 验证和客户端运行验证。

客户端配置路径为 `${XDG_CONFIG_HOME:-$HOME/.config}/wincast/client.toml`。`XDG_CONFIG_HOME` 必须是非空绝对路径；未设置、为空或为相对路径时，程序按 `$HOME/.config/wincast/client.toml` 查找。

Client 配置重点检查：

- `host` 是 Windows host 可达 IP 或主机名。
- `port` 与 Windows host 的 `listen` 端口一致。
- x86_64 与 aarch64/ARM64 目标机分别使用对应架构产物。

## 烟测前检查

进入真机烟测前，至少确认：

- Windows host 能直接运行 `wincast-host` 并读取默认配置。
- Linux client 能直接运行 `wincast-client` 并读取默认配置。
- 两端在同一可达网络内，端口从 Linux client 到 Windows host 可达。
- 待捕获程序可以在 Windows host 前台正常启动。
- 当前版本已完成本地 Rust 门禁；如果门禁因为网络、依赖下载或环境问题未通过，需要在烟测记录中如实说明。

完成以上准备后，再执行 `docs/smoke-test.md`，并分别记录 Windows host、Linux x86_64 client 和 Linux aarch64/ARM64 client 的结果。
