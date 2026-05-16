# wincast-media 第三方库选型记录

## 当前结论

本 crate 当前已接入 `openh264` 作为稳定版前台链路的 H.264 软件编解码后端，并继续把后端细节限制在 `wincast-media` 内部。

原因：

- `wincast-protocol` 已经定义 `EncodedVideoFrame` 和 `VideoCodec`，媒体 crate 应复用协议边界，避免提前把某个库的帧模型扩散到网络协议。
- OpenH264 比 FFmpeg、GStreamer 和平台硬编更轻，能先满足 Windows host 前台编码与 Linux client 解码的最小运行链路。
- FFmpeg、GStreamer 和平台硬编仍涉及跨 Windows、Linux x86_64、Linux aarch64 的构建、部署、动态库和授权边界，在没有完成更完整适配验证前不进入当前稳定版默认链路。
- 测试继续固定 H.264、1080p、fps 和 bitrate 参数边界，保证当前稳定链路不被后端细节污染。

## 已接入后端

- OpenH264：当前默认软件 H.264 编解码后端。宿主端把 BGRA 捕获帧转为 YUV 后编码为 H.264，客户端解码后转回 BGRA 交给现有渲染器。该后端依赖 `openh264-sys2` 在构建时编译 C/C++ 源码，因此目标构建环境必须具备可用 C/C++ 工具链。

## 非默认候选

- GStreamer、FFmpeg/libavcodec 和 Windows Media Foundation 不进入当前稳定版默认链路。
- 如因目标机 OpenH264 部署失败而必须替换后端，应另行形成明确变更，不把替换事项写成当前项目路线图。
- 任何替换都不能改变上层协议仍只传输 H.264 编码帧这一边界。

## 接入门槛

任一新增后端进入稳定版默认链路前，需要先有明确变更决策，并证明：

- 能实现 `VideoEncoder` 或 `VideoDecoder`，并只在 crate 内暴露后端细节。
- 输出或消费 `wincast_protocol::message::EncodedVideoFrame`。
- 支持 H.264、最高 1920x1080、默认 30 FPS、低延迟参数。
- 在目标平台上能给出清晰构建和部署方式。

## 参考资料

- GStreamer `x264enc`: https://gstreamer.freedesktop.org/documentation/x264/index.html
- FFmpeg `libavcodec`: https://ffmpeg.org/doxygen/trunk/group__libavc.html
- OpenH264: https://github.com/cisco/openh264
- Microsoft Media Foundation H.264 encoder: https://learn.microsoft.com/windows/win32/medfound/h-264-video-encoder
