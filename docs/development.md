# 开发说明

## Rust 版本

工程使用 Rust 2024 edition，最低 Rust 版本为 1.88。

## 常用验证

```powershell
cargo fmt --all
cargo test --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## 客户端 ARM64 目标

客户端需要覆盖 Linux x86_64 与 Linux aarch64/ARM64：

```powershell
rustup target add aarch64-unknown-linux-gnu
cargo check -p wincast-client --target aarch64-unknown-linux-gnu
```

当前客户端第一阶段使用 SDL2 创建 Linux 窗口并显示 raw BGRA 帧。银河麒麟 V10 等目标机需要安装 SDL2 开发包后再执行 Linux 目标构建；当前 Windows 开发机上的 workspace 验证只能证明非 Linux 占位路径和协议逻辑可编译，不能替代目标系统真机构建。
