use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputInjectionError {
    InvalidCaptureBounds,
    InvalidMouseCoordinate,
    UnsupportedKeyCode(u32),
    #[cfg(not(windows))]
    UnsupportedPlatform,
    #[cfg(windows)]
    WindowsSendInputFailed,
}

impl fmt::Display for InputInjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCaptureBounds => {
                formatter.write_str("输入映射失败：捕获区域宽高必须大于 0")
            }
            Self::InvalidMouseCoordinate => {
                formatter.write_str("输入映射失败：鼠标坐标必须是有效数字")
            }
            Self::UnsupportedKeyCode(code) => {
                write!(
                    formatter,
                    "输入映射失败：按键码 {code} 超出 Windows virtual-key 范围"
                )
            }
            #[cfg(not(windows))]
            Self::UnsupportedPlatform => {
                formatter.write_str("当前平台不支持输入注入：仅 Windows 支持 SendInput")
            }
            #[cfg(windows)]
            Self::WindowsSendInputFailed => {
                formatter.write_str("Windows 输入注入失败：SendInput 未接受全部输入事件")
            }
        }
    }
}

impl std::error::Error for InputInjectionError {}
