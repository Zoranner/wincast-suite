# wincast-media 第三方库选型记录

## 当前结论

本 crate 当前已接入 `openh264` 作为稳定版前台链路的 H.264 软件编解码后端，并继续把后端细节限制在 `wincast-media` 内部。

原因：

- `wincast-protocol` 已经定义 `EncodedVideoFrame` 和 `VideoCodec`，媒体 crate 应复用协议边界，避免提前把某个库的帧模型扩散到网络协议。
- OpenH264 比 FFmpeg、GStreamer 和平台硬编更轻，能先满足 Windows host 前台编码与 Linux client 解码的最小运行链路。
- FFmpeg、GStreamer 和平台硬编仍涉及跨 Windows、Linux x86_64、Linux aarch64 的构建、部署、动态库和授权边界，在没有完成更完整适配验证前不进入当前稳定版默认链路。
- 测试继续固定 H.264、1080p、fps 和 bitrate 参数边界，保证后续后端替换不影响上层调用者。

## 已接入后端

- OpenH264：当前默认软件 H.264 编解码后端。宿主端把 BGRA 捕获帧转为 YUV 后编码为 H.264，客户端解码后转回 BGRA 交给现有渲染器。该后端依赖 `openh264-sys2` 在构建时编译 C/C++ 源码，因此目标构建环境必须具备可用 C/C++ 工具链。

## 后续候选

- GStreamer：优先评估跨平台 pipeline 能力、Linux ARM64 包可得性、H.264 低延迟参数、硬解/软解切换成本。官方 `x264enc` 文档明确 `tune=zerolatency` 可降低编码缓冲延迟，但会影响整体编码质量，需要实测取舍。
- FFmpeg/libavcodec：优先评估 Windows 与 Linux 的打包方式、动态库部署、H.264 编解码参数控制和许可证边界。它适合作为完整 codec API 候选，但不应直接污染上层协议类型。
- Windows Media Foundation：优先评估 Windows 宿主端硬件编码路径。官方 H.264 encoder 是 Media Foundation Transform，输出类型为 `MFVideoFormat_H264`，但它只覆盖 Windows 宿主端，不作为 Linux 客户端解码方案。

## 接入门槛

任一新增后端进入稳定版默认链路前，需要先证明：

- 能实现 `VideoEncoder` 或 `VideoDecoder`，并只在 crate 内暴露后端细节。
- 输出或消费 `wincast_protocol::message::EncodedVideoFrame`。
- 支持 H.264、最高 1920x1080、默认 30 FPS、低延迟参数。
- 在目标平台上能给出清晰构建和部署方式。

## 参考资料

- GStreamer `x264enc`: https://gstreamer.freedesktop.org/documentation/x264/index.html
- FFmpeg `libavcodec`: https://ffmpeg.org/doxygen/trunk/group__libavc.html
- OpenH264: https://github.com/cisco/openh264
- Microsoft Media Foundation H.264 encoder: https://learn.microsoft.com/windows/win32/medfound/h-264-video-encoder
