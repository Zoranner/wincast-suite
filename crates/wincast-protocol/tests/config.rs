use wincast_protocol::config::{
    ClientConfig, ConfigError, HostBackendMode, HostConfig, MonitorPowerAfterLaunch, VideoCodec,
};

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
    let config = HostConfig::from_toml_str(&example_config("host.toml"))
        .expect("stable host example config should parse");

    assert_eq!(config.program.path, "C:\\Windows\\System32\\notepad.exe");
    assert_eq!(config.program.work_dir, "C:\\Windows\\System32");
    assert_eq!(config.program.startup_delay_ms, 3000);
    assert_eq!(config.capture.first_frame_timeout_ms, 5000);
    assert_eq!(config.video.codec, VideoCodec::H264);
}

#[test]
fn parses_stable_client_example_config() {
    let config = ClientConfig::from_toml_str(&example_config("client.toml"))
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

[program]
path = 'C:\Program Files\SomeApp\app.exe'
args = ["--demo"]
work_dir = 'C:\Program Files\SomeApp'
startup_delay_ms = 0

[video]
width = 1280
height = 720
fps = 30
codec = "h264"
bitrate_kbps = 4000
max_bitrate_kbps = 6000

[capture]
first_frame_timeout_ms = 5000
"#,
    )
    .expect("host config should parse");

    assert_eq!(config.listen, "0.0.0.0:7856");
    assert_eq!(config.mode, HostBackendMode::DesktopDxgi);
    assert_eq!(config.program.path, "C:\\Program Files\\SomeApp\\app.exe");
    assert_eq!(config.program.args, ["--demo"]);
    assert_eq!(config.program.work_dir, "C:\\Program Files\\SomeApp");
    assert_eq!(config.program.startup_delay_ms, 0);
    assert_eq!(
        config.program.turn_off_monitor_after_launch,
        MonitorPowerAfterLaunch::Disabled
    );
    assert_eq!(config.video.width, 1280);
    assert_eq!(config.video.codec, VideoCodec::H264);
    assert_eq!(config.capture.first_frame_timeout_ms, 5000);
}

#[test]
fn parses_unity_embedded_host_config() {
    let config = HostConfig::from_toml_str(
        r#"
listen = "0.0.0.0:7856"
mode = "unity_embedded"

[program]
path = 'C:\Program Files\SomeUnityApp\UnityApp.exe'
args = ["--profile", "demo"]
work_dir = 'C:\Program Files\SomeUnityApp'
startup_delay_ms = 0

[unity]
executable = 'C:\Program Files\SomeUnityApp\UnityApp.exe'
work_dir = 'C:\Program Files\SomeUnityApp'
port = 7900

[video]
width = 1280
height = 720
fps = 30
codec = "h264"
bitrate_kbps = 4000
max_bitrate_kbps = 6000

[capture]
first_frame_timeout_ms = 5000
"#,
    )
    .expect("unity embedded host config should parse");

    assert_eq!(config.mode, HostBackendMode::UnityEmbedded);
    let unity = config
        .unity
        .expect("unity config should be present for embedded backend");
    assert_eq!(
        unity.executable,
        "C:\\Program Files\\SomeUnityApp\\UnityApp.exe"
    );
    assert_eq!(unity.port, 7900);
}

#[test]
fn rejects_unity_embedded_host_config_without_unity_section() {
    let error = HostConfig::from_toml_str(
        r#"
listen = "0.0.0.0:7856"
mode = "unity_embedded"

[program]
path = 'C:\Program Files\SomeUnityApp\UnityApp.exe'
work_dir = 'C:\Program Files\SomeUnityApp'
startup_delay_ms = 0

[video]
width = 1280
height = 720
fps = 30
codec = "h264"
bitrate_kbps = 4000
max_bitrate_kbps = 6000

[capture]
first_frame_timeout_ms = 5000
"#,
    )
    .expect_err("unity embedded mode should require unity section");

    assert_eq!(error, ConfigError::MissingField("unity"));
}

#[test]
fn rejects_unity_embedded_host_config_with_zero_port() {
    let error = HostConfig::from_toml_str(
        r#"
listen = "0.0.0.0:7856"
mode = "unity_embedded"

[program]
path = 'C:\Program Files\SomeUnityApp\UnityApp.exe'
work_dir = 'C:\Program Files\SomeUnityApp'
startup_delay_ms = 0

[unity]
executable = 'C:\Program Files\SomeUnityApp\UnityApp.exe'
work_dir = 'C:\Program Files\SomeUnityApp'
port = 0

[video]
width = 1280
height = 720
fps = 30
codec = "h264"
bitrate_kbps = 4000
max_bitrate_kbps = 6000

[capture]
first_frame_timeout_ms = 5000
"#,
    )
    .expect_err("unity port should be validated");

    assert_eq!(
        error,
        ConfigError::InvalidValue {
            field: "unity.port",
            reason: "必须大于 0",
        }
    );
}

#[test]
fn rejects_legacy_unity_embedded_host_config_with_port_range() {
    let error = HostConfig::from_toml_str(
        r#"
listen = "0.0.0.0:7856"
mode = "unity_embedded"

[program]
path = 'C:\Program Files\SomeUnityApp\UnityApp.exe'
work_dir = 'C:\Program Files\SomeUnityApp'
startup_delay_ms = 0

[unity]
executable = 'C:\Program Files\SomeUnityApp\UnityApp.exe'
work_dir = 'C:\Program Files\SomeUnityApp'
instance_port_start = 7900
instance_port_end = 7999
max_instances = 4

[video]
width = 1280
height = 720
fps = 30
codec = "h264"
bitrate_kbps = 4000
max_bitrate_kbps = 6000

[capture]
first_frame_timeout_ms = 5000
"#,
    )
    .expect_err("legacy unity port range should be rejected");

    assert!(matches!(error, ConfigError::InvalidToml(_)));
}

#[test]
fn parses_host_config_with_ddc_ci_monitor_power_strategy() {
    let config = HostConfig::from_toml_str(
        r#"
listen = "0.0.0.0:7856"

[program]
path = 'C:\Program Files\SomeApp\app.exe'
work_dir = 'C:\Program Files\SomeApp'
startup_delay_ms = 3000
turn_off_monitor_after_launch = "ddc_ci_dim"

[video]
width = 1280
height = 720
fps = 30
codec = "h264"
bitrate_kbps = 4000
max_bitrate_kbps = 6000

[capture]
first_frame_timeout_ms = 5000
"#,
    )
    .expect("host config with DDC/CI monitor power strategy should parse");

    assert_eq!(
        config.program.turn_off_monitor_after_launch,
        MonitorPowerAfterLaunch::DdcCiDim
    );
}

#[test]
fn rejects_windows_power_message_monitor_power_strategy() {
    let source = r#"
listen = "127.0.0.1:47011"

[program]
path = "C:\\Windows\\System32\\notepad.exe"
args = []
work_dir = "C:\\Windows\\System32"
startup_delay_ms = 100
turn_off_monitor_after_launch = "windows_power_message"

[video]
width = 1280
height = 720
fps = 30
codec = "h264"
bitrate_kbps = 4000
max_bitrate_kbps = 8000

[capture]
first_frame_timeout_ms = 1000
"#;

    let error = HostConfig::from_toml_str(source)
        .expect_err("Windows monitor power message should be rejected");

    assert_eq!(
        error,
        ConfigError::InvalidValue {
            field: "program.turn_off_monitor_after_launch",
            reason: "真正关闭显示器会破坏 DXGI Desktop Duplication 画面捕获；请使用 disabled 或 ddc_ci_dim",
        }
    );
}

#[test]
fn rejects_ddc_ci_power_off_monitor_power_strategy() {
    let source = r#"
listen = "127.0.0.1:47011"

[program]
path = "C:\\Windows\\System32\\notepad.exe"
args = []
work_dir = "C:\\Windows\\System32"
startup_delay_ms = 100
turn_off_monitor_after_launch = "ddc_ci_power_off"

[video]
width = 1280
height = 720
fps = 30
codec = "h264"
bitrate_kbps = 4000
max_bitrate_kbps = 8000

[capture]
first_frame_timeout_ms = 1000
"#;

    let error =
        HostConfig::from_toml_str(source).expect_err("DDC/CI monitor power off should be rejected");

    assert_eq!(
        error,
        ConfigError::InvalidValue {
            field: "program.turn_off_monitor_after_launch",
            reason: "真正关闭显示器会破坏 DXGI Desktop Duplication 画面捕获；请使用 disabled 或 ddc_ci_dim",
        }
    );
}

#[test]
fn rejects_boolean_monitor_power_config() {
    let err = HostConfig::from_toml_str(
        r#"
listen = "0.0.0.0:7856"

[program]
path = 'C:\Program Files\SomeApp\app.exe'
work_dir = 'C:\Program Files\SomeApp'
startup_delay_ms = 3000
turn_off_monitor_after_launch = true

[video]
width = 1280
height = 720
fps = 30
codec = "h264"
bitrate_kbps = 4000
max_bitrate_kbps = 6000

[capture]
first_frame_timeout_ms = 5000
"#,
    )
    .expect_err("boolean monitor power config should no longer parse");

    assert!(matches!(err, ConfigError::InvalidToml(_)));
}

#[test]
fn rejects_unknown_monitor_power_strategy() {
    let err = HostConfig::from_toml_str(
        r#"
listen = "0.0.0.0:7856"

[program]
path = 'C:\Program Files\SomeApp\app.exe'
work_dir = 'C:\Program Files\SomeApp'
startup_delay_ms = 3000
turn_off_monitor_after_launch = "sleep"

[video]
width = 1280
height = 720
fps = 30
codec = "h264"
bitrate_kbps = 4000
max_bitrate_kbps = 6000

[capture]
first_frame_timeout_ms = 5000
"#,
    )
    .expect_err("unknown monitor power strategy should be rejected");

    assert!(matches!(err, ConfigError::InvalidToml(_)));
}

#[test]
fn rejects_legacy_capture_mode_config() {
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
max_bitrate_kbps = 6000

[capture]
mode = "display"
startup_timeout_ms = 15000
"#,
    )
    .expect_err("legacy host config should no longer parse");

    assert!(matches!(err, ConfigError::InvalidToml(_)));
}

#[test]
fn rejects_host_config_with_raw_bgra_video_codec() {
    let err = HostConfig::from_toml_str(
        r#"
listen = "0.0.0.0:7856"

[program]
path = 'C:\Program Files\SomeApp\app.exe'
work_dir = 'C:\Program Files\SomeApp'
startup_delay_ms = 3000

[video]
width = 1280
height = 720
fps = 30
codec = "raw_bgra"
bitrate_kbps = 4000
max_bitrate_kbps = 6000

[capture]
first_frame_timeout_ms = 5000
"#,
    )
    .expect_err("raw BGRA is not a configurable video codec");

    assert!(matches!(err, ConfigError::InvalidToml(_)));
}

#[test]
fn parses_host_config_with_h264_video_codec() {
    let config = HostConfig::from_toml_str(
        r#"
listen = "0.0.0.0:7856"

[program]
path = 'C:\Program Files\SomeApp\app.exe'
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

[program]
path = 'C:\Program Files\SomeApp\app.exe'
work_dir = 'C:\Program Files\SomeApp'
startup_delay_ms = 3000

[video]
width = 1280
height = 720
fps = 30
codec = "vp9"
bitrate_kbps = 4000
max_bitrate_kbps = 6000

[capture]
first_frame_timeout_ms = 5000
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
fn rejects_host_config_with_empty_program_path() {
    let err = HostConfig::from_toml_str(
        r#"
listen = "0.0.0.0:7856"

[program]
path = ""
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
"#,
    )
    .expect_err("empty program path should be rejected");

    assert_eq!(err, ConfigError::MissingField("program.path"));
}

#[test]
fn rejects_host_config_with_zero_first_frame_timeout() {
    let err = HostConfig::from_toml_str(
        r#"
listen = "0.0.0.0:7856"

[program]
path = 'C:\Program Files\SomeApp\app.exe'
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
first_frame_timeout_ms = 0
"#,
    )
    .expect_err("zero first frame timeout should be rejected");

    assert_eq!(
        err,
        ConfigError::InvalidValue {
            field: "capture.first_frame_timeout_ms",
            reason: "必须大于 0",
        }
    );
}

#[test]
fn rejects_host_config_with_invalid_video_dimensions() {
    let err = HostConfig::from_toml_str(
        r#"
listen = "0.0.0.0:7856"

[program]
path = 'C:\Program Files\SomeApp\app.exe'
work_dir = 'C:\Program Files\SomeApp'
startup_delay_ms = 3000

[video]
width = 0
height = 720
fps = 30
codec = "h264"
bitrate_kbps = 4000
max_bitrate_kbps = 6000

[capture]
first_frame_timeout_ms = 5000
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
