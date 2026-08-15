//! Node 与 DSH CLI 运行时路径解析。

use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_READINESS_TIMEOUT: Duration = Duration::from_secs(90);
const CLI_RELATIVE_PATH: &str = "node_modules/@deepseek-ai/dsh/lib/bin.js";

/// 启动 Host 所需的全部已解析路径与超时配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePaths {
    pub node: PathBuf,
    pub cli_entry: PathBuf,
    pub working_directory: PathBuf,
    pub readiness_timeout: Duration,
}

impl RuntimePaths {
    /// 按环境覆盖、应用资源和本机安装的顺序解析原型运行时。
    pub fn resolve(resource_dir: &Path) -> Result<Self, String> {
        Ok(Self {
            node: resolve_node(resource_dir),
            cli_entry: resolve_cli_entry(resource_dir)?,
            working_directory: resolve_working_directory(),
            readiness_timeout: resolve_readiness_timeout(),
        })
    }
}

/// 解析 Node 可执行文件，优先使用显式覆盖和应用内置副本。
fn resolve_node(resource_dir: &Path) -> PathBuf {
    if let Some(value) = non_empty_env("DSH_DESKTOP_NODE_EXECUTABLE") {
        return PathBuf::from(value);
    }
    let bundled = resource_dir.join("node/node.exe");
    if bundled.is_file() {
        return bundled;
    }
    find_in_path("node.exe").unwrap_or_else(|| PathBuf::from("node"))
}

/// 解析 DSH CLI 入口，并在所有候选路径都不存在时返回可诊断错误。
fn resolve_cli_entry(resource_dir: &Path) -> Result<PathBuf, String> {
    if let Some(value) = non_empty_env("DSH_DESKTOP_CLI_ENTRY") {
        let path = PathBuf::from(&value);
        return path.is_file().then_some(path).ok_or_else(|| {
            format!("DSH_DESKTOP_CLI_ENTRY is set but the file does not exist: {value}")
        });
    }

    let mut candidates = vec![
        resource_dir.join("host").join(CLI_RELATIVE_PATH),
        PathBuf::from(r"C:\Program Files\DeepSeek Harness\resources\host").join(CLI_RELATIVE_PATH),
    ];
    if let Some(node) = find_in_path("node.exe") {
        if let Some(directory) = node.parent() {
            candidates.push(directory.join(CLI_RELATIVE_PATH));
        }
    }

    candidates.iter().find(|path| path.is_file()).cloned().ok_or_else(|| {
        format!(
            "could not locate the dsh CLI entry (looked in: {}). Set DSH_DESKTOP_CLI_ENTRY to @deepseek-ai/dsh/lib/bin.js.",
            candidates
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })
}

/// 解析 Host 工作目录，默认使用当前用户主目录。
fn resolve_working_directory() -> PathBuf {
    non_empty_env("DSH_DESKTOP_CWD")
        .or_else(|| non_empty_env("USERPROFILE"))
        .or_else(|| non_empty_env("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 解析启动超时；无效或空值回退到 90 秒。
fn resolve_readiness_timeout() -> Duration {
    non_empty_env("DSH_DESKTOP_READY_TIMEOUT_SECS")
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_READINESS_TIMEOUT)
}

/// 读取去除首尾空白后仍非空的环境变量。
fn non_empty_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// 在 PATH 中查找指定可执行文件。
fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}
