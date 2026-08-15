//! 桌面壳与 Host 的文件日志。

use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// 返回当前用户可写的桌面应用日志路径。
pub fn log_file_path() -> PathBuf {
    let base = env::var("LOCALAPPDATA")
        .or_else(|_| env::var("TEMP"))
        .unwrap_or_else(|_| ".".into());
    let dir = Path::new(&base).join("dsh-desktop");
    let _ = fs::create_dir_all(&dir);
    dir.join("dsh-desktop.log")
}

/// 追加一行桌面应用日志；日志失败不得阻断主流程。
pub fn log_app(message: &str) {
    append("app", message);
}

/// 追加一行 Host 标准输出或错误输出。
pub fn log_host(message: &str) {
    append("host", message);
}

/// 将带来源标签的日志追加到统一文件。
fn append(source: &str, message: &str) {
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file_path())
    {
        let _ = writeln!(file, "[{source}] {message}");
    }
}
