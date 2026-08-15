//! 带脱敏和轮转的桌面壳文件日志。

use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::SecondsFormat;
use regex::Regex;

pub const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;
const LOG_FILE_COUNT: usize = 3;

static LOG_LOCK: Mutex<()> = Mutex::new(());
static SECRET_PATTERN: OnceLock<Regex> = OnceLock::new();
static BEARER_PATTERN: OnceLock<Regex> = OnceLock::new();

/// 日志消息的严重级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Error,
}

impl LogLevel {
    /// 返回稳定、便于检索的日志级别名称。
    fn as_str(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Error => "ERROR",
        }
    }
}

/// 返回当前用户可写的桌面应用日志路径。
pub fn log_file_path() -> PathBuf {
    if let Some(directory) = env::var_os("DSH_DESKTOP_LOG_DIR").filter(|value| !value.is_empty()) {
        let directory = PathBuf::from(directory);
        let _ = fs::create_dir_all(&directory);
        return directory.join("dsh-desktop.log");
    }
    let base = env::var("LOCALAPPDATA")
        .or_else(|_| env::var("TEMP"))
        .unwrap_or_else(|_| ".".into());
    let dir = Path::new(&base).join("dsh-desktop");
    let _ = fs::create_dir_all(&dir);
    dir.join("dsh-desktop.log")
}

/// 追加一行普通桌面应用日志。
pub fn log_app(message: &str) {
    append(LogLevel::Info, "app", message);
}

/// 追加一行桌面应用错误日志。
pub fn log_error(message: &str) {
    append(LogLevel::Error, "app", message);
}

/// 追加一行 Host 标准输出或错误输出。
pub fn log_host(message: &str) {
    append(LogLevel::Info, "host", message);
}

/// 将日志写入指定文件，超过 5 MiB 时仅保留最近三个文件。
fn append(level: LogLevel, source: &str, message: &str) {
    let _guard = LOG_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let path = log_file_path();
    let timestamp = chrono::Local::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let sanitized = redact_secrets(message);
    let line = format!(
        "{timestamp} level={} pid={} source={source} {sanitized}\n",
        level.as_str(),
        std::process::id()
    );
    if let Err(error) = rotate_if_needed(&path, line.len() as u64) {
        eprintln!("failed to rotate desktop log: {error}");
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(line.as_bytes());
    }
}

/// 替换常见认证字段和 Bearer 凭据，避免 Host 输出泄露到诊断日志。
pub fn redact_secrets(message: &str) -> String {
    let secrets = SECRET_PATTERN.get_or_init(|| {
        Regex::new(r#"(?i)\b(token|authorization|api[_-]?key|password)([\s\"':=]+)([^\s,;]+)"#)
            .expect("secret redaction pattern must compile")
    });
    let bearer = BEARER_PATTERN.get_or_init(|| {
        Regex::new(r"(?i)\bBearer\s+[^\s,;]+").expect("bearer redaction pattern must compile")
    });
    // 先整体替换 Bearer 值，避免通用字段规则只吃掉 `Bearer` 而遗留真实凭据。
    let without_bearer = bearer.replace_all(message, "Bearer [REDACTED]");
    secrets
        .replace_all(&without_bearer, "$1$2[REDACTED]")
        .into_owned()
}

/// 在追加内容会超过上限时执行从旧到新的确定性日志轮转。
fn rotate_if_needed(path: &Path, incoming_bytes: u64) -> std::io::Result<()> {
    let current_bytes = path.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    if current_bytes.saturating_add(incoming_bytes) <= MAX_LOG_BYTES {
        return Ok(());
    }

    let oldest = rotated_path(path, LOG_FILE_COUNT - 1);
    if oldest.exists() {
        fs::remove_file(&oldest)?;
    }
    for index in (1..LOG_FILE_COUNT - 1).rev() {
        let source = rotated_path(path, index);
        if source.exists() {
            fs::rename(source, rotated_path(path, index + 1))?;
        }
    }
    if path.exists() {
        fs::rename(path, rotated_path(path, 1))?;
    }
    Ok(())
}

/// 生成 `dsh-desktop.log.1` 形式的轮转文件路径。
fn rotated_path(path: &Path, index: usize) -> PathBuf {
    PathBuf::from(format!("{}.{index}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{redact_secrets, rotate_if_needed, rotated_path, MAX_LOG_BYTES};

    #[test]
    fn redacts_common_secret_formats() {
        let line = "Authorization: Bearer abc123 password=hunter2 api_key='secret' token: xyz";
        let redacted = redact_secrets(line);
        assert!(!redacted.contains("abc123"));
        assert!(!redacted.contains("hunter2"));
        assert!(!redacted.contains("secret"));
        assert!(!redacted.contains("xyz"));
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn rotates_at_five_mib_and_keeps_three_files() {
        let directory = std::env::temp_dir().join(format!(
            "dsh-desktop-log-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("dsh-desktop.log");

        for generation in 1..=3 {
            let file = fs::File::create(&path).unwrap();
            file.set_len(MAX_LOG_BYTES).unwrap();
            rotate_if_needed(&path, 1).unwrap();
            assert!(rotated_path(&path, 1).exists(), "generation {generation}");
        }

        assert!(path.with_file_name("dsh-desktop.log.1").exists());
        assert!(path.with_file_name("dsh-desktop.log.2").exists());
        assert!(!path.with_file_name("dsh-desktop.log.3").exists());
        fs::remove_dir_all(directory).unwrap();
    }
}
