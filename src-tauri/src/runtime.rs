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
    /// 解析桌面运行时；开发构建允许环境覆盖，发布构建只接受内置资源。
    pub fn resolve(resource_dir: &Path) -> Result<Self, String> {
        RuntimeInputs::from_environment().resolve(resource_dir, cfg!(debug_assertions))
    }
}

/// 将环境读取与路径决策分离，测试可以注入确定性输入。
#[derive(Debug, Default)]
struct RuntimeInputs {
    node_override: Option<String>,
    cli_override: Option<String>,
    working_directory: Option<String>,
    user_profile: Option<String>,
    home: Option<String>,
    readiness_timeout: Option<String>,
    path_directories: Vec<PathBuf>,
}

impl RuntimeInputs {
    /// 从当前进程环境收集原始配置，不在这一层做路径选择。
    fn from_environment() -> Self {
        Self {
            node_override: non_empty_env("DSH_DESKTOP_NODE_EXECUTABLE"),
            cli_override: non_empty_env("DSH_DESKTOP_CLI_ENTRY"),
            working_directory: non_empty_env("DSH_DESKTOP_CWD"),
            user_profile: non_empty_env("USERPROFILE"),
            home: non_empty_env("HOME"),
            readiness_timeout: non_empty_env("DSH_DESKTOP_READY_TIMEOUT_SECS"),
            path_directories: env::var_os("PATH")
                .map(|path| env::split_paths(&path).collect())
                .unwrap_or_default(),
        }
    }

    /// 根据已捕获输入生成完整运行时路径。
    fn resolve(
        &self,
        resource_dir: &Path,
        allow_development_fallbacks: bool,
    ) -> Result<RuntimePaths, String> {
        let resource_dir = normalize_windows_verbatim_path(resource_dir);
        Ok(RuntimePaths {
            node: self.resolve_node(&resource_dir, allow_development_fallbacks)?,
            cli_entry: self.resolve_cli_entry(&resource_dir, allow_development_fallbacks)?,
            working_directory: self.resolve_working_directory(),
            readiness_timeout: self.resolve_readiness_timeout(),
        })
    }

    /// 解析 Node 可执行文件，优先使用显式覆盖和应用内置副本。
    fn resolve_node(
        &self,
        resource_dir: &Path,
        allow_development_fallbacks: bool,
    ) -> Result<PathBuf, String> {
        if allow_development_fallbacks {
            if let Some(value) = &self.node_override {
                return Ok(PathBuf::from(value));
            }
        }
        let bundled = resource_dir.join("node/node.exe");
        if bundled.is_file() {
            return Ok(bundled);
        }
        if allow_development_fallbacks {
            return Ok(self
                .find_in_path("node.exe")
                .unwrap_or_else(|| PathBuf::from("node")));
        }
        Err(format!(
            "bundled Node runtime is missing: {}",
            bundled.display()
        ))
    }

    /// 解析 DSH CLI；release 模式只接受安装包携带的固定入口。
    fn resolve_cli_entry(
        &self,
        resource_dir: &Path,
        allow_development_fallbacks: bool,
    ) -> Result<PathBuf, String> {
        if allow_development_fallbacks {
            if let Some(value) = &self.cli_override {
                let path = PathBuf::from(value);
                return path.is_file().then_some(path).ok_or_else(|| {
                    format!("DSH_DESKTOP_CLI_ENTRY is set but the file does not exist: {value}")
                });
            }
        }

        let bundled = resource_dir.join("host").join(CLI_RELATIVE_PATH);
        if bundled.is_file() {
            return Ok(bundled);
        }
        if !allow_development_fallbacks {
            return Err(format!(
                "bundled DSH CLI entry is missing: {}",
                bundled.display()
            ));
        }

        let mut candidates =
            vec![
                PathBuf::from(r"C:\Program Files\DeepSeek Harness\resources\host")
                    .join(CLI_RELATIVE_PATH),
            ];
        if let Some(node) = self.find_in_path("node.exe") {
            if let Some(directory) = node.parent() {
                candidates.push(directory.join(CLI_RELATIVE_PATH));
            }
        }

        candidates
            .iter()
            .find(|path| path.is_file())
            .cloned()
            .ok_or_else(|| {
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
    fn resolve_working_directory(&self) -> PathBuf {
        self.working_directory
            .as_ref()
            .or(self.user_profile.as_ref())
            .or(self.home.as_ref())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// 解析启动超时；无效或空值回退到 90 秒。
    fn resolve_readiness_timeout(&self) -> Duration {
        self.readiness_timeout
            .as_ref()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_READINESS_TIMEOUT)
    }

    /// 在已注入的 PATH 目录中查找指定可执行文件。
    fn find_in_path(&self, name: &str) -> Option<PathBuf> {
        self.path_directories
            .iter()
            .map(|directory| directory.join(name))
            .find(|candidate| candidate.is_file())
    }
}

/// 将 Windows verbatim 路径转为普通 Win32 路径，避免 Node 把 CLI 参数误解析为盘符。
fn normalize_windows_verbatim_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let value = path.as_os_str().to_string_lossy();
        if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = value.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    path.to_path_buf()
}

/// 读取去除首尾空白后仍非空的环境变量。
fn non_empty_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use super::{RuntimeInputs, CLI_RELATIVE_PATH, DEFAULT_READINESS_TIMEOUT};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "dsh-desktop-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn file(&self, relative: &str) -> PathBuf {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"test").unwrap();
            path
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn bundled_runtime_has_priority_over_path() {
        let resources = TestDirectory::new("runtime-bundled");
        let path_runtime = TestDirectory::new("runtime-path");
        let bundled_node = resources.file("node/node.exe");
        let bundled_cli = resources.file(&format!("host/{CLI_RELATIVE_PATH}"));
        path_runtime.file("node.exe");
        path_runtime.file(CLI_RELATIVE_PATH);

        let inputs = RuntimeInputs {
            working_directory: Some(resources.path().display().to_string()),
            path_directories: vec![path_runtime.path().to_owned()],
            ..Default::default()
        };
        let resolved = inputs.resolve(resources.path(), true).unwrap();
        assert_eq!(resolved.node, bundled_node);
        assert_eq!(resolved.cli_entry, bundled_cli);
        assert_eq!(resolved.working_directory, resources.path());
        assert_eq!(resolved.readiness_timeout, DEFAULT_READINESS_TIMEOUT);
    }

    #[test]
    fn explicit_overrides_and_timeout_are_applied() {
        let resources = TestDirectory::new("runtime-overrides");
        let node = resources.file("custom/node.exe");
        let cli = resources.file("custom/bin.js");
        let inputs = RuntimeInputs {
            node_override: Some(node.display().to_string()),
            cli_override: Some(cli.display().to_string()),
            working_directory: Some(resources.path().display().to_string()),
            readiness_timeout: Some("12".to_owned()),
            ..Default::default()
        };
        let resolved = inputs.resolve(resources.path(), true).unwrap();
        assert_eq!(resolved.node, node);
        assert_eq!(resolved.cli_entry, cli);
        assert_eq!(resolved.readiness_timeout, Duration::from_secs(12));
    }

    #[test]
    fn invalid_cli_override_and_timeout_are_diagnostic() {
        let resources = TestDirectory::new("runtime-invalid");
        let inputs = RuntimeInputs {
            cli_override: Some(resources.path().join("missing.js").display().to_string()),
            readiness_timeout: Some("invalid".to_owned()),
            ..Default::default()
        };
        let error = inputs.resolve(resources.path(), true).unwrap_err();
        assert!(error.contains("does not exist"));

        let timeout_inputs = RuntimeInputs {
            readiness_timeout: Some("invalid".to_owned()),
            ..Default::default()
        };
        assert_eq!(
            timeout_inputs.resolve_readiness_timeout(),
            DEFAULT_READINESS_TIMEOUT
        );
    }

    #[test]
    fn release_mode_requires_bundled_runtime_and_ignores_overrides() {
        let resources = TestDirectory::new("runtime-release");
        let overrides = TestDirectory::new("runtime-release-overrides");
        let node_override = overrides.file("node.exe");
        let cli_override = overrides.file("bin.js");
        let inputs = RuntimeInputs {
            node_override: Some(node_override.display().to_string()),
            cli_override: Some(cli_override.display().to_string()),
            ..Default::default()
        };
        assert!(inputs.resolve(resources.path(), false).is_err());

        let bundled_node = resources.file("node/node.exe");
        let bundled_cli = resources.file(&format!("host/{CLI_RELATIVE_PATH}"));
        let resolved = inputs.resolve(resources.path(), false).unwrap();
        assert_eq!(resolved.node, bundled_node);
        assert_eq!(resolved.cli_entry, bundled_cli);
    }

    #[cfg(windows)]
    #[test]
    fn release_mode_removes_windows_verbatim_prefix_from_runtime_paths() {
        let resources = TestDirectory::new("runtime-verbatim");
        let bundled_node = resources.file("node/node.exe");
        let bundled_cli = resources.file(&format!("host/{CLI_RELATIVE_PATH}"));
        let verbatim_resources = PathBuf::from(format!(r"\\?\{}", resources.path().display()));

        let resolved = RuntimeInputs::default()
            .resolve(&verbatim_resources, false)
            .unwrap();
        assert_eq!(resolved.node, bundled_node);
        assert_eq!(resolved.cli_entry, bundled_cli);
    }
}
