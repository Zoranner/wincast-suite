# wincast-media 第三方库选型记录

## 当前结论

本 crate 先只建立媒体边界，不直接引入 FFmpeg、GStreamer、openh264 或平台硬编绑定。

原因：

- 当前任务目标是稳定 host/client 之外的媒体抽象，不是接入运行时链路。
- `wincast-protocol` 已经定义 `EncodedVideoFrame` 和 `VideoCodec`，媒体 crate 应复用协议边界，避免提前把某个库的帧模型扩散到网络协议。
- FFmpeg、GStreamer 和平台硬编都涉及跨 Windows、Linux x86_64、Linux aarch64 的构建、部署、动态库和授权边界；在没有完成最小适配验证前，不应把重依赖写进 workspace。
- 第一阶段需要先用测试固定 H.264、1080p、fps 和 bitrate 参数边界，保证后续后端替换不影响上层调用者。

## 下一步候选

- GStreamer：优先评估跨平台 pipeline 能力、Linux ARM64 包可得性、H.264 低延迟参数、硬解/软解切换成本。官方 `x264enc` 文档明确 `tune=zerolatency` 可降低编码缓冲延迟，但会影响整体编码质量，需要实测取舍。
- FFmpeg/libavcodec：优先评估 Windows 与 Linux 的打包方式、动态库部署、H.264 编解码参数控制和许可证边界。它适合作为完整 codec API 候选，但不应直接污染上层协议类型。
- OpenH264：优先评估轻量软件 H.264 编解码、码率控制能力、延迟表现和 1080p 负载。官方项目定位就是 H.264 encoder/decoder，适合做轻量后端候选。
- Windows Media Foundation：优先评估 Windows 宿主端硬件编码路径。官方 H.264 encoder 是 Media Foundation Transform，输出类型为 `MFVideoFormat_H264`，但它只覆盖 Windows 宿主端，不作为 Linux 客户端解码方案。

## 接入门槛

任一后端进入 workspace 前，需要先证明：

- 能实现 `VideoEncoder` 或 `VideoDecoder`，并只在 crate 内暴露后端细节。
- 输出或消费 `wincast_protocol::message::EncodedVideoFrame`。
- 支持 H.264、最高 1920x1080、默认 30 FPS、低延迟参数。
- 在目标平台上能给出清晰构建和部署方式。

## 参考资料

- GStreamer `x264enc`: https://gstreamer.freedesktop.org/documentation/x264/index.html
- FFmpeg `libavcodec`: https://ffmpeg.org/doxygen/trunk/group__libavc.html
- OpenH264: https://github.com/cisco/openh264
- Microsoft Media Foundation H.264 encoder: https://learn.microsoft.com/windows/win32/medfound/h-264-video-encoder
