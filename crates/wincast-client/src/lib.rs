use std::{env, path::PathBuf, time::Duration};

mod errors;
mod render_loop;
mod runtime;
mod stream;

#[cfg(test)]
mod runtime_tests;

const DEFAULT_RETRIES: u32 = 3;
const DEFAULT_RETRY_DELAY_MS: u64 = 1_000;

pub fn run_default_client() -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        return runtime::run_fullscreen_client(&default_config_path(), default_retry_options());
    }

    #[cfg(not(target_os = "linux"))]
    {
        runtime::run_client(&default_config_path(), default_retry_options())
    }
}

pub fn run_headless_client() -> Result<String, String> {
    runtime::run_client(&default_config_path(), default_retry_options())
}

fn default_retry_options() -> runtime::RetryOptions {
    runtime::RetryOptions {
        retries: DEFAULT_RETRIES,
        retry_delay: Duration::from_millis(DEFAULT_RETRY_DELAY_MS),
    }
}

fn default_config_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                env::var_os("USERPROFILE")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("AppData")
                    .join("Roaming")
            })
            .join("WinCast")
            .join("client.toml")
    }

    #[cfg(not(target_os = "windows"))]
    {
        xdg_config_path(env::var_os("XDG_CONFIG_HOME"), env::var_os("HOME"))
    }
}

#[cfg(any(test, not(target_os = "windows")))]
fn xdg_config_path(
    xdg_config_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> PathBuf {
    xdg_config_home
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| {
            home.map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config")
        })
        .join("wincast")
        .join("client.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_uses_default_retry_policy_without_user_arguments() {
        let options = default_retry_options();

        assert_eq!(options.retries, 3);
        assert_eq!(options.retry_delay, Duration::from_millis(1_000));
    }

    #[test]
    fn xdg_config_path_falls_back_when_xdg_config_home_is_empty() {
        let home = absolute_test_home();
        let path = xdg_config_path(Some("".into()), Some(home.as_os_str().into()));

        assert_eq!(path, expected_client_config_under_home(&home));
    }

    #[test]
    fn xdg_config_path_ignores_relative_xdg_config_home() {
        let home = absolute_test_home();
        let path = xdg_config_path(
            Some("relative-config".into()),
            Some(home.as_os_str().into()),
        );

        assert_eq!(path, expected_client_config_under_home(&home));
    }

    fn absolute_test_home() -> PathBuf {
        #[cfg(windows)]
        {
            PathBuf::from(r"C:\Users\tester")
        }

        #[cfg(not(windows))]
        {
            PathBuf::from("/home/tester")
        }
    }

    fn expected_client_config_under_home(home: &std::path::Path) -> PathBuf {
        home.join(".config").join("wincast").join("client.toml")
    }
}
