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

当前客户端未引入平台 GUI 或媒体依赖，因此 ARM64 检查只验证协议、配置和 CLI 骨架可编译。接入渲染、解码或系统媒体库的改动必须同步验证目标系统的开发包和链接器配置。
