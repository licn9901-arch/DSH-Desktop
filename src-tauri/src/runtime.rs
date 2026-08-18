//! Node 与 DSH CLI 运行时路径解析。

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::payload::{read_runtime_state, RuntimeSlot};

const DEFAULT_CORE_READY_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_PLUGIN_READY_TIMEOUT: Duration = Duration::from_secs(30);
const CLI_RELATIVE_PATH: &str = "node_modules/@deepseek-ai/dsh/lib/bin.js";

/// 启动 Host 所需的全部已解析路径与超时配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePaths {
    pub node: PathBuf,
    pub cli_entry: PathBuf,
    pub host_root: PathBuf,
    pub tool_bin_directory: PathBuf,
    pub desktop_policy_patch: PathBuf,
    pub plugins_root: PathBuf,
    pub user_home: PathBuf,
    pub dsh_home: PathBuf,
    pub web_profile: PathBuf,
    pub managed_plugins_root: PathBuf,
    pub working_directory: PathBuf,
    pub core_ready_timeout: Duration,
    pub plugin_ready_timeout: Duration,
    pub immutable_plugins: bool,
    pub activation: Option<RuntimeActivation>,
}

/// 记录当前路径是否来自 payload candidate，供就绪后提升或失败回退。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeActivation {
    pub runtime_root: PathBuf,
    pub payload_digest: String,
    pub runtime_abi: u32,
    pub candidate: bool,
}

/// 启动时解析出的主运行时和唯一回退运行时。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupRuntimeSelection {
    pub primary: RuntimePaths,
    pub fallback: Option<RuntimePaths>,
}

impl RuntimePaths {
    /// 解析桌面运行时；开发构建允许环境覆盖，发布构建只接受内置资源。
    pub fn resolve(resource_dir: &Path) -> Result<Self, String> {
        Ok(Self::resolve_startup(resource_dir)?.primary)
    }

    /// 解析 payload candidate/active 及回退关系；没有状态时保持 legacy 行为。
    pub fn resolve_startup(resource_dir: &Path) -> Result<StartupRuntimeSelection, String> {
        RuntimeInputs::from_environment().resolve_startup(resource_dir, cfg!(debug_assertions))
    }
}

/// 返回当前用户桌面托管 runtime 根目录；正常启动不接受 release 环境覆盖。
pub fn default_runtime_root() -> Result<PathBuf, String> {
    let local_app_data = non_empty_env("LOCALAPPDATA")
        .ok_or_else(|| "LOCALAPPDATA is not available for the managed runtime".to_owned())?;
    Ok(PathBuf::from(local_app_data)
        .join("dsh-desktop")
        .join("runtime"))
}

/// 返回安装器 smoke 专用的 WebView2 数据目录；正常发布启动不读取任意目录覆盖。
pub fn test_webview_data_directory() -> Result<Option<PathBuf>, String> {
    let Some(value) = non_empty_env("DSH_DESKTOP_WEBVIEW_TEST_DATA_DIR") else {
        return Ok(None);
    };
    validate_test_webview_data_directory(Path::new(&value), &env::temp_dir()).map(Some)
}

/// 只接受系统临时目录下固定两层结构，canonicalize 同时拒绝不存在目录和链接逃逸。
fn validate_test_webview_data_directory(
    data_directory: &Path,
    temp_root: &Path,
) -> Result<PathBuf, String> {
    let canonical_temp = fs::canonicalize(temp_root)
        .map_err(|error| format!("could not canonicalize system temp directory: {error}"))?;
    let canonical_data = fs::canonicalize(data_directory).map_err(|error| {
        format!(
            "could not canonicalize WebView test data directory {}: {error}",
            data_directory.display()
        )
    })?;
    let relative = canonical_data.strip_prefix(&canonical_temp).map_err(|_| {
        format!(
            "WebView test data directory is outside the system temp directory: {}",
            canonical_data.display()
        )
    })?;
    let components = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let valid_prefix = components.first().is_some_and(|component| {
        let component = component.to_ascii_lowercase();
        component.starts_with("dsh-desktop-installer-smoke-")
            || component.starts_with("dsh-desktop-smoke-")
    });
    if components.len() != 2 || !valid_prefix || !components[1].eq_ignore_ascii_case("webview-data")
    {
        return Err(format!(
            "WebView test data directory has an unexpected path shape: {}",
            canonical_data.display()
        ));
    }
    Ok(canonical_data)
}

/// 将环境读取与路径决策分离，测试可以注入确定性输入。
#[derive(Debug, Default)]
struct RuntimeInputs {
    node_override: Option<String>,
    cli_override: Option<String>,
    working_directory: Option<String>,
    user_home_override: Option<String>,
    user_profile: Option<String>,
    home: Option<String>,
    dsh_home: Option<String>,
    readiness_timeout: Option<String>,
    core_ready_timeout: Option<String>,
    plugin_ready_timeout: Option<String>,
    path_directories: Vec<PathBuf>,
    local_app_data: Option<String>,
    runtime_root: Option<PathBuf>,
}

impl RuntimeInputs {
    /// 从当前进程环境收集原始配置，不在这一层做路径选择。
    fn from_environment() -> Self {
        Self {
            node_override: non_empty_env("DSH_DESKTOP_NODE_EXECUTABLE"),
            cli_override: non_empty_env("DSH_DESKTOP_CLI_ENTRY"),
            working_directory: non_empty_env("DSH_DESKTOP_CWD"),
            user_home_override: non_empty_env("DSH_DESKTOP_USER_HOME"),
            user_profile: non_empty_env("USERPROFILE"),
            home: non_empty_env("HOME"),
            dsh_home: non_empty_env("DSH_HOME"),
            readiness_timeout: non_empty_env("DSH_DESKTOP_READY_TIMEOUT_SECS"),
            core_ready_timeout: non_empty_env("DSH_DESKTOP_CORE_READY_TIMEOUT_SECS"),
            plugin_ready_timeout: non_empty_env("DSH_DESKTOP_PLUGIN_READY_TIMEOUT_SECS"),
            path_directories: env::var_os("PATH")
                .map(|path| env::split_paths(&path).collect())
                .unwrap_or_default(),
            local_app_data: non_empty_env("LOCALAPPDATA"),
            runtime_root: if cfg!(debug_assertions) {
                non_empty_env("DSH_DESKTOP_RUNTIME_ROOT").map(PathBuf::from)
            } else {
                None
            },
        }
    }

    /// 根据单状态文件选择 candidate、active 或 legacy 资源。
    fn resolve_startup(
        &self,
        resource_dir: &Path,
        allow_development_fallbacks: bool,
    ) -> Result<StartupRuntimeSelection, String> {
        let runtime_root = self.runtime_root.clone().or_else(|| {
            self.local_app_data
                .as_ref()
                .map(|path| Path::new(path).join("dsh-desktop/runtime"))
        });
        let Some(runtime_root) = runtime_root else {
            return Ok(StartupRuntimeSelection {
                primary: self.resolve(resource_dir, allow_development_fallbacks)?,
                fallback: None,
            });
        };
        let state = read_runtime_state(&runtime_root)?;
        if let Some(candidate) = &state.candidate {
            let primary = self.resolve_payload_slot(&runtime_root, candidate, true)?;
            let fallback = state
                .active
                .as_ref()
                .map(|active| self.resolve_payload_slot(&runtime_root, active, false))
                .transpose()?;
            return Ok(StartupRuntimeSelection { primary, fallback });
        }
        if let Some(active) = &state.active {
            return Ok(StartupRuntimeSelection {
                primary: self.resolve_payload_slot(&runtime_root, active, false)?,
                fallback: None,
            });
        }
        Ok(StartupRuntimeSelection {
            primary: self.resolve(resource_dir, allow_development_fallbacks)?,
            fallback: None,
        })
    }

    /// 将摘要状态映射为 runtime 根目录下的固定路径，并关闭开发回退。
    fn resolve_payload_slot(
        &self,
        runtime_root: &Path,
        slot: &RuntimeSlot,
        candidate: bool,
    ) -> Result<RuntimePaths, String> {
        slot.validate()?;
        let mut paths = self.resolve(&runtime_root.join(&slot.payload_digest), false)?;
        paths.immutable_plugins = true;
        paths.activation = Some(RuntimeActivation {
            runtime_root: runtime_root.to_owned(),
            payload_digest: slot.payload_digest.clone(),
            runtime_abi: slot.runtime_abi,
            candidate,
        });
        Ok(paths)
    }

    /// 根据已捕获输入生成完整运行时路径。
    fn resolve(
        &self,
        resource_dir: &Path,
        allow_development_fallbacks: bool,
    ) -> Result<RuntimePaths, String> {
        let resource_dir = normalize_windows_verbatim_path(resource_dir);
        let user_home = self.resolve_user_home(allow_development_fallbacks);
        let dsh_home = self.resolve_dsh_home();
        Ok(RuntimePaths {
            node: self.resolve_node(&resource_dir, allow_development_fallbacks)?,
            cli_entry: self.resolve_cli_entry(&resource_dir, allow_development_fallbacks)?,
            host_root: resource_dir.join("host"),
            tool_bin_directory: resource_dir.join("host/node_modules/.bin"),
            desktop_policy_patch: resource_dir.join("policy/dsh-market.patch.yml"),
            plugins_root: resource_dir.join("plugins"),
            user_home,
            web_profile: dsh_home.join("profiles/web"),
            managed_plugins_root: dsh_home.join("profiles/node_modules/.dsh-desktop"),
            dsh_home,
            working_directory: self.resolve_working_directory(),
            core_ready_timeout: self.resolve_core_ready_timeout(),
            plugin_ready_timeout: self.resolve_plugin_ready_timeout(),
            immutable_plugins: false,
            activation: None,
        })
    }

    /// 解析真实用户主目录，供不遵循 DSH_HOME 的第三方工具保存首次配置。
    fn resolve_user_home(&self, allow_development_fallbacks: bool) -> PathBuf {
        if allow_development_fallbacks {
            if let Some(value) = &self.user_home_override {
                return PathBuf::from(value);
            }
        }
        self.user_profile
            .as_ref()
            .or(self.home.as_ref())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
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

    /// 解析 DSH 用户数据根目录，并与 Host 启动时注入的 `DSH_HOME` 保持一致。
    fn resolve_dsh_home(&self) -> PathBuf {
        self.dsh_home
            .as_ref()
            .map(PathBuf::from)
            .or_else(|| {
                self.user_profile
                    .as_ref()
                    .map(|home| Path::new(home).join(".dsh"))
            })
            .or_else(|| self.home.as_ref().map(|home| Path::new(home).join(".dsh")))
            .unwrap_or_else(|| PathBuf::from(".dsh"))
    }

    /// 解析核心就绪超时；旧变量作为兼容回退，默认 60 秒。
    fn resolve_core_ready_timeout(&self) -> Duration {
        parse_timeout(self.core_ready_timeout.as_ref())
            .or_else(|| parse_timeout(self.readiness_timeout.as_ref()))
            .unwrap_or(DEFAULT_CORE_READY_TIMEOUT)
    }

    /// 解析插件就绪超时；旧变量作为兼容回退，默认 30 秒。
    fn resolve_plugin_ready_timeout(&self) -> Duration {
        parse_timeout(self.plugin_ready_timeout.as_ref())
            .or_else(|| parse_timeout(self.readiness_timeout.as_ref()))
            .unwrap_or(DEFAULT_PLUGIN_READY_TIMEOUT)
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

/// 将正整数秒转换为 Duration，零值和非法输入均视为未配置。
fn parse_timeout(value: Option<&String>) -> Option<Duration> {
    value
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use super::{
        validate_test_webview_data_directory, RuntimeInputs, CLI_RELATIVE_PATH,
        DEFAULT_CORE_READY_TIMEOUT, DEFAULT_PLUGIN_READY_TIMEOUT,
    };
    use crate::payload::{write_runtime_state, RuntimeSlot, RuntimeState};

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

    #[test]
    fn webview_test_data_directory_accepts_only_the_installer_smoke_shape() {
        let temp = TestDirectory::new("webview-validation-temp");
        let valid_root = temp.path().join("dsh-desktop-installer-smoke-fixture");
        let valid = valid_root.join("webview-data");
        fs::create_dir_all(&valid).unwrap();
        assert_eq!(
            validate_test_webview_data_directory(&valid, temp.path()).unwrap(),
            fs::canonicalize(&valid).unwrap()
        );

        let wrong_leaf = valid_root.join("runtime");
        fs::create_dir_all(&wrong_leaf).unwrap();
        assert!(validate_test_webview_data_directory(&wrong_leaf, temp.path()).is_err());

        let wrong_prefix = temp.path().join("untrusted-smoke/webview-data");
        fs::create_dir_all(&wrong_prefix).unwrap();
        assert!(validate_test_webview_data_directory(&wrong_prefix, temp.path()).is_err());
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
        assert_eq!(resolved.core_ready_timeout, DEFAULT_CORE_READY_TIMEOUT);
        assert_eq!(resolved.plugin_ready_timeout, DEFAULT_PLUGIN_READY_TIMEOUT);
    }

    #[test]
    fn default_core_ready_timeout_allows_slow_cold_start() {
        assert_eq!(
            RuntimeInputs::default().resolve_core_ready_timeout(),
            Duration::from_secs(60)
        );
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
            user_home_override: Some(resources.path().join("user").display().to_string()),
            readiness_timeout: Some("12".to_owned()),
            core_ready_timeout: Some("7".to_owned()),
            plugin_ready_timeout: Some("19".to_owned()),
            ..Default::default()
        };
        let resolved = inputs.resolve(resources.path(), true).unwrap();
        assert_eq!(resolved.node, node);
        assert_eq!(resolved.cli_entry, cli);
        assert_eq!(resolved.user_home, resources.path().join("user"));
        assert_eq!(resolved.core_ready_timeout, Duration::from_secs(7));
        assert_eq!(resolved.plugin_ready_timeout, Duration::from_secs(19));
    }

    #[test]
    fn legacy_timeout_is_the_compatibility_fallback_for_both_stages() {
        let inputs = RuntimeInputs {
            readiness_timeout: Some("12".to_owned()),
            ..Default::default()
        };
        assert_eq!(inputs.resolve_core_ready_timeout(), Duration::from_secs(12));
        assert_eq!(
            inputs.resolve_plugin_ready_timeout(),
            Duration::from_secs(12)
        );
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
            timeout_inputs.resolve_core_ready_timeout(),
            DEFAULT_CORE_READY_TIMEOUT
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
            user_home_override: Some(overrides.path().join("user").display().to_string()),
            ..Default::default()
        };
        assert!(inputs.resolve(resources.path(), false).is_err());

        let bundled_node = resources.file("node/node.exe");
        let bundled_cli = resources.file(&format!("host/{CLI_RELATIVE_PATH}"));
        let resolved = inputs.resolve(resources.path(), false).unwrap();
        assert_eq!(resolved.node, bundled_node);
        assert_eq!(resolved.cli_entry, bundled_cli);
        assert_ne!(resolved.user_home, overrides.path().join("user"));
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

    #[test]
    fn explicit_dsh_home_wins_and_profile_paths_are_derived_from_it() {
        let resources = TestDirectory::new("runtime-dsh-home-resources");
        let home = TestDirectory::new("runtime-dsh-home");
        resources.file("node/node.exe");
        resources.file(&format!("host/{CLI_RELATIVE_PATH}"));
        resources.file("plugins/plugins.lock.json");

        let inputs = RuntimeInputs {
            dsh_home: Some(home.path().display().to_string()),
            user_profile: Some("ignored-user-profile".to_owned()),
            ..Default::default()
        };
        let resolved = inputs.resolve(resources.path(), false).unwrap();
        assert_eq!(resolved.dsh_home, home.path());
        assert_eq!(resolved.web_profile, home.path().join("profiles/web"));
        assert_eq!(resolved.host_root, resources.path().join("host"));
        assert_eq!(
            resolved.tool_bin_directory,
            resources.path().join("host/node_modules/.bin")
        );
        assert_eq!(
            resolved.desktop_policy_patch,
            resources.path().join("policy/dsh-market.patch.yml")
        );
        assert_eq!(resolved.plugins_root, resources.path().join("plugins"));
    }

    #[test]
    fn default_dsh_home_uses_user_profile_dot_dsh() {
        let resources = TestDirectory::new("runtime-default-dsh-home-resources");
        let user = TestDirectory::new("runtime-default-dsh-home-user");
        resources.file("node/node.exe");
        resources.file(&format!("host/{CLI_RELATIVE_PATH}"));
        resources.file("plugins/plugins.lock.json");

        let resolved = RuntimeInputs {
            user_profile: Some(user.path().display().to_string()),
            ..Default::default()
        }
        .resolve(resources.path(), false)
        .unwrap();
        assert_eq!(resolved.dsh_home, user.path().join(".dsh"));
    }

    #[test]
    fn startup_prefers_candidate_and_keeps_active_as_fallback() {
        let resources = TestDirectory::new("runtime-selection-resources");
        let runtime_root = TestDirectory::new("runtime-selection-root");
        resources.file("node/node.exe");
        resources.file(&format!("host/{CLI_RELATIVE_PATH}"));
        let active_digest = "a".repeat(64);
        let candidate_digest = "b".repeat(64);
        for digest in [&active_digest, &candidate_digest] {
            runtime_root.file(&format!("{digest}/node/node.exe"));
            runtime_root.file(&format!("{digest}/host/{CLI_RELATIVE_PATH}"));
            fs::create_dir_all(
                runtime_root
                    .path()
                    .join(digest)
                    .join("plugins/node_modules"),
            )
            .unwrap();
        }
        write_runtime_state(
            runtime_root.path(),
            &RuntimeState {
                schema_version: 1,
                active: Some(RuntimeSlot::new(&active_digest, 1, "old")),
                previous: None,
                candidate: Some(RuntimeSlot::new(&candidate_digest, 1, "new")),
            },
        )
        .unwrap();

        let selection = RuntimeInputs {
            runtime_root: Some(runtime_root.path().to_owned()),
            ..Default::default()
        }
        .resolve_startup(resources.path(), false)
        .unwrap();

        assert_eq!(
            selection
                .primary
                .activation
                .as_ref()
                .unwrap()
                .payload_digest,
            candidate_digest
        );
        assert!(selection.primary.activation.as_ref().unwrap().candidate);
        assert!(selection.primary.immutable_plugins);
        let fallback = selection.fallback.expect("candidate 必须保留 active 回退");
        assert_eq!(
            fallback.activation.as_ref().unwrap().payload_digest,
            active_digest
        );
        assert!(!fallback.activation.as_ref().unwrap().candidate);
    }

    #[test]
    fn startup_uses_legacy_resources_when_no_payload_state_exists() {
        let resources = TestDirectory::new("runtime-selection-legacy");
        let runtime_root = TestDirectory::new("runtime-selection-empty");
        resources.file("node/node.exe");
        resources.file(&format!("host/{CLI_RELATIVE_PATH}"));

        let selection = RuntimeInputs {
            runtime_root: Some(runtime_root.path().to_owned()),
            ..Default::default()
        }
        .resolve_startup(resources.path(), false)
        .unwrap();

        assert!(selection.primary.activation.is_none());
        assert!(!selection.primary.immutable_plugins);
        assert!(selection.fallback.is_none());
    }
}
