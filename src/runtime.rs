use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMode {
    /// cargo run または cargo watch -x run から起動
    CargoRun,
    /// target/debug/ 直下のデバッグバイナリを直接起動
    DebugBinary,
    /// 最適化済み release バイナリを直接起動
    ReleaseBinary,
}

impl RuntimeMode {
    pub fn detect() -> Self {
        let is_cargo_env = env::var("CARGO").is_ok() || env::var("CARGO_MANIFEST_DIR").is_ok();

        if is_cargo_env {
            Self::CargoRun
        } else if cfg!(debug_assertions) {
            Self::DebugBinary
        } else {
            Self::ReleaseBinary
        }
    }

    #[inline]
    pub fn is_dev(&self) -> bool {
        matches!(self, Self::CargoRun | Self::DebugBinary)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::CargoRun => "DEV (cargo run)",
            Self::DebugBinary => "DEBUG",
            Self::ReleaseBinary => "RELEASE",
        }
    }

    /// release バイナリの推奨パス（存在する場合）
    pub fn get_preferred_release_exe() -> Option<PathBuf> {
        let current = env::current_exe().ok()?;
        // すでに release バイナリならそれ自身
        let path_str = current.to_string_lossy();
        if path_str.contains("target\\release") || !path_str.contains("target\\debug") {
            return Some(current);
        }

        // target/debug 内にいる場合、兄弟の target/release を探す
        if let Some(target_dir) = current.parent().and_then(|p| p.parent()) {
            let release_exe = target_dir.join("release").join(current.file_name()?);
            if release_exe.exists() {
                return Some(release_exe);
            }
        }

        Some(current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_mode_detect() {
        let mode = RuntimeMode::detect();
        // テスト環境は debug プロファイル
        assert!(mode.is_dev());
    }
}
