use wincast_protocol::config::{CaptureMode, ClientConfig, ConfigError, HostConfig, VideoCodec};

fn example_config(name: &str) -> String {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = manifest_dir
        .join("..")
        .join("..")
        .join("examples")
        .join(name);

    std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read example config {}: {}", path.display(), err))
}

#[test]
fn parses_stable_host_example_config() {
    let config = HostConfig::from_toml_str(&example_config("wincast-host.toml"))
        .expect("stable host example config should parse");

    assert_eq!(config.capture.mode, CaptureMode::Window);
    assert_eq!(config.video.codec, VideoCodec::RawBgra);
    assert!(
        !config.capture.window_title_contains.trim().is_empty(),
        "stable host example must keep a non-empty window title hint"
    );
}

#[test]
fn parses_stable_client_example_config() {
    let config = ClientConfig::from_toml_str(&example_config("wincast-client.toml"))
        .expect("stable client example config should parse");

    assert!(
        !config.host.trim().is_empty(),
        "stable client example must keep a non-empty host"
    );
    assert_ne!(
        config.port, 0,
        "stable client example must keep a non-zero port"
    );
}

#[test]
fn parses_host_config_from_documented_toml() {
    let config = HostConfig::from_toml_str(
        r#"
listen = "0.0.0.0:7856"
program = "C:\\Program Files\\SomeApp\\app.exe"
args = ["--demo"]
work_dir = "C:\\Program Files\\SomeApp"

[video]
width = 1280
height = 720
fps = 30
codec = "h264"
bitrate_kbps = 4000

[capture]
mode = "window"
window_title_contains = "SomeApp"
startup_timeout_ms = 15000
"#,
    )
    .expect("host config should parse");

    assert_eq!(config.listen, "0.0.0.0:7856");
    assert_eq!(config.args, ["--demo"]);
    assert_eq!(config.video.width, 1280);
    assert_eq!(config.video.codec, VideoCodec::H264);
    assert_eq!(config.capture.mode, CaptureMode::Window);
    assert_eq!(config.capture.window_title_contains, "SomeApp");
}

#[test]
fn parses_host_config_with_raw_bgra_video_codec() {
    let config = HostConfig::from_toml_str(
        r#"
listen = "0.0.0.0:7856"
program = "C:\\Program Files\\SomeApp\\app.exe"
work_dir = "C:\\Program Files\\SomeApp"

[video]
width = 1280
height = 720
fps = 30
codec = "raw_bgra"
bitrate_kbps = 4000

[capture]
mode = "desktop"
startup_timeout_ms = 15000
"#,
    )
    .expect("raw BGRA host config should parse");

    assert_eq!(config.video.codec, VideoCodec::RawBgra);
}

#[test]
fn parses_host_config_with_h264_video_codec() {
    let config = HostConfig::from_toml_str(
        r#"
listen = "0.0.0.0:7856"
program = "C:\\Program Files\\SomeApp\\app.exe"
work_dir = "C:\\Program Files\\SomeApp"

[video]
width = 1280
height = 720
fps = 30
codec = "h264"
bitrate_kbps = 4000

[capture]
mode = "desktop"
startup_timeout_ms = 15000
"#,
    )
    .expect("H.264 host config should parse");

    assert_eq!(config.video.codec, VideoCodec::H264);
}

#[test]
fn rejects_unknown_video_codec_as_invalid_toml() {
    let err = HostConfig::from_toml_str(
        r#"
listen = "0.0.0.0:7856"
program = "C:\\Program Files\\SomeApp\\app.exe"
work_dir = "C:\\Program Files\\SomeApp"

[video]
width = 1280
height = 720
fps = 30
codec = "vp9"
bitrate_kbps = 4000

[capture]
mode = "desktop"
startup_timeout_ms = 15000
"#,
    )
    .expect_err("unknown codec should be rejected by TOML deserialization");

    assert!(matches!(err, ConfigError::InvalidToml(_)));
}

#[test]
fn parses_client_config_and_formats_endpoint() {
    let config = ClientConfig::from_toml_str(
        r#"
host = "192.168.10.25"
port = 7856
"#,
    )
    .expect("client config should parse");

    assert_eq!(config.endpoint(), "192.168.10.25:7856");
}

#[test]
fn rejects_host_config_with_empty_program() {
    let err = HostConfig::from_toml_str(
        r#"
listen = "0.0.0.0:7856"
program = ""
work_dir = "C:\\Program Files\\SomeApp"

[video]
width = 1280
height = 720
fps = 30
codec = "h264"
bitrate_kbps = 4000

[capture]
mode = "desktop"
startup_timeout_ms = 15000
"#,
    )
    .expect_err("empty program should be rejected");

    assert_eq!(err, ConfigError::MissingField("program"));
}

#[test]
fn rejects_host_config_with_invalid_video_dimensions() {
    let err = HostConfig::from_toml_str(
        r#"
listen = "0.0.0.0:7856"
program = "C:\\Program Files\\SomeApp\\app.exe"
work_dir = "C:\\Program Files\\SomeApp"

[video]
width = 0
height = 720
fps = 30
codec = "h264"
bitrate_kbps = 4000

[capture]
mode = "desktop"
startup_timeout_ms = 15000
"#,
    )
    .expect_err("zero width should be rejected");

    assert_eq!(
        err,
        ConfigError::InvalidValue {
            field: "video.width",
            reason: "必须大于 0",
        }
    );
}

#[test]
fn rejects_window_capture_without_title_hint() {
    let err = HostConfig::from_toml_str(
        r#"
listen = "0.0.0.0:7856"
program = "C:\\Program Files\\SomeApp\\app.exe"
work_dir = "C:\\Program Files\\SomeApp"

[video]
width = 1280
height = 720
fps = 30
codec = "h264"
bitrate_kbps = 4000

[capture]
mode = "window"
startup_timeout_ms = 15000
"#,
    )
    .expect_err("window capture needs a title hint");

    assert_eq!(
        err,
        ConfigError::InvalidValue {
            field: "capture.window_title_contains",
            reason: "窗口捕获模式必须配置窗口标题匹配文本",
        }
    );
}

#[test]
fn rejects_client_config_with_empty_host() {
    let err = ClientConfig::from_toml_str(
        r#"
host = ""
port = 7856
"#,
    )
    .expect_err("empty host should be rejected");

    assert_eq!(err, ConfigError::MissingField("host"));
}

#[test]
fn rejects_invalid_toml() {
    let err = ClientConfig::from_toml_str("host = [").expect_err("toml should be invalid");

    assert!(matches!(err, ConfigError::InvalidToml(_)));
}
