//! 桌面托管插件的锁文件、profile 迁移和失败回滚。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::runtime::RuntimePaths;

const SUPPORTED_SCHEMA_VERSION: u32 = 1;
const BASE_BUNDLE: &str = "@deepseek-ai/dsh-base";
const WEB_APP_BUNDLE: &str = "@deepseek-ai/dsh-web-app";
const MARKET_BUNDLE: &str = "dshmarket";
const MARKET_RUNTIME_ALIAS: &str = "dshmarket-desktop";
const RUNTIME_SERVICES_BUNDLE: &str = "@dsh-desktop/runtime-services";
const DESKTOP_SETTINGS_BUNDLE: &str = "@dsh-desktop/theme-settings";
const LEGACY_SKILLS_MCP_BUNDLE: &str = "@zebbkira/dsh-skills-mcp-manager";
const SKILLS_MCP_BUNDLE: &str = "@cubee-slide/skills-mcp-manager";
const LEGACY_SIDE_PANEL: &str = "@dsh-external/dsh-side-panel";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 描述构建期已验证、运行期允许挂载的全部插件。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginLock {
    pub schema_version: u32,
    pub plugins: Vec<ManagedPlugin>,
    #[serde(default)]
    pub shared_packages: Vec<String>,
    #[serde(default)]
    pub transitive_packages: Vec<ManagedDependency>,
    #[serde(default)]
    pub skills: Vec<ManagedSkill>,
}

/// 描述一个固定版本插件及运行前必须存在的文件。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManagedPlugin {
    pub package: String,
    pub version: String,
    pub bundle_id: String,
    pub license: String,
    pub source: PluginSource,
    #[serde(default)]
    pub required_files: Vec<String>,
}

/// 描述插件构建输入的固定来源与完整性凭据。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum PluginSource {
    /// npm 正式发布物使用 SRI SHA-512 绑定。
    Npm { integrity: String },
    /// GitHub tag 归档同时绑定 URL、目标 commit 与归档 SHA-256。
    GithubTarball {
        url: String,
        commit: String,
        sha256: String,
    },
    /// GitHub Release 中的 npm pack 绑定仓库、tag、commit 与文件 SHA-256。
    GithubReleaseAsset {
        repository: String,
        tag: String,
        commit: String,
        url: String,
        sha256: String,
    },
    /// 仓库自有预构建 bundle，只允许从仓库内相对目录暂存。
    Local { path: String },
}

/// 描述从锁定插件发布物复制到 DSH 用户级目录的托管 Skill。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManagedSkill {
    pub name: String,
    pub source_package: String,
    pub source_file: String,
    pub version: String,
    pub sha256: String,
}

/// 描述需要与主插件一起建立 profile junction、但不直接激活 bundle 的依赖。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManagedDependency {
    pub package: String,
    pub version: String,
    pub license: String,
    pub integrity: String,
    #[serde(default)]
    pub required_files: Vec<String>,
}

/// 记录由桌面端创建的插件链接，避免覆盖用户自行安装的包。
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginInstallState {
    pub schema_version: u32,
    pub lock_digest: String,
    #[serde(default)]
    pub managed: BTreeMap<String, ManagedPluginState>,
    #[serde(default)]
    pub managed_skills: BTreeMap<String, ManagedSkillState>,
    #[serde(default)]
    pub sidebar_defaults_seeded: bool,
}

/// 记录托管 Skill 上次成功写入的摘要，并记住用户主动删除的选择。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManagedSkillState {
    pub version: String,
    pub content_sha256: String,
    #[serde(default)]
    pub user_removed: bool,
}

/// 托管 Skill 的纯规划结果，文件写入仍由插件事务执行。
#[derive(Debug, Clone, PartialEq, Eq)]
enum ManagedSkillAction {
    Write(ManagedSkillState),
    KeepManaged(ManagedSkillState),
    PreserveUnmanaged,
    RememberRemoved(ManagedSkillState),
}

/// 记录单个桌面托管插件上次成功安装的状态。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManagedPluginState {
    pub version: String,
    pub link_target: String,
    pub bundle_enabled: bool,
}

/// profile 迁移的纯计算结果，文件系统操作由事务层统一执行。
#[derive(Debug, Clone)]
struct ProfilePlan {
    profile: Value,
    next_state: PluginInstallState,
    managed_packages: Vec<String>,
    removed_packages: Vec<String>,
}

/// 抽象 Windows directory junction，测试可注入内存实现。
trait DirectoryLinker: Send + Sync {
    /// 返回链接当前指向的真实目录；普通目录或不存在时返回 `None`。
    fn target(&self, link: &Path) -> Result<Option<PathBuf>, String>;
    /// 创建一个只指向已验证目标的目录链接。
    fn create(&self, link: &Path, target: &Path) -> Result<(), String>;
    /// 删除链接本身，不删除目标目录。
    fn remove(&self, link: &Path) -> Result<(), String>;
}

/// 使用 Windows junction 挂载插件，避免普通用户依赖开发者模式。
struct SystemDirectoryLinker {
    node: PathBuf,
}

/// 安装前准备桌面托管插件，并返回可提交或回滚的事务。
pub struct PluginManager {
    resources: PathBuf,
    dsh_home: PathBuf,
    web_profile: PathBuf,
    managed_plugins_root: PathBuf,
    bundled_market_root: PathBuf,
    legacy_market_root: PathBuf,
    user_home: PathBuf,
    linker: Arc<dyn DirectoryLinker>,
}

/// 保存本次 profile 和链接变更；Host 就绪后提交，失败时恢复。
pub struct PluginTransaction {
    should_seed_sidebar: bool,
    state_path: PathBuf,
    next_state: PluginInstallState,
    snapshots: Vec<FileSnapshot>,
    link_changes: Vec<LinkChange>,
    linker: Arc<dyn DirectoryLinker>,
    finalized: bool,
}

/// 保存文件变更前的原始字节；`None` 表示文件原先不存在。
struct FileSnapshot {
    path: PathBuf,
    original: Option<Vec<u8>>,
}

/// 记录一次目录链接替换，用于按逆序恢复旧目标。
struct LinkChange {
    link: PathBuf,
    previous_target: Option<PathBuf>,
}

impl PluginManager {
    /// 从已解析的运行时路径创建生产环境插件管理器。
    pub fn new(paths: &RuntimePaths) -> Self {
        Self::with_linker(
            paths.plugins_root.clone(),
            paths.dsh_home.clone(),
            paths.web_profile.clone(),
            paths.managed_plugins_root.clone(),
            paths.host_root.join("node_modules/dshmarket"),
            paths.user_home.clone(),
            Arc::new(SystemDirectoryLinker {
                node: paths.node.clone(),
            }),
        )
    }

    /// 使用显式路径和链接实现创建管理器，供应用与测试共享。
    fn with_linker(
        resources: PathBuf,
        dsh_home: PathBuf,
        web_profile: PathBuf,
        managed_plugins_root: PathBuf,
        bundled_market_root: PathBuf,
        user_home: PathBuf,
        linker: Arc<dyn DirectoryLinker>,
    ) -> Self {
        let legacy_market_root = bundled_market_root.with_file_name(MARKET_RUNTIME_ALIAS);
        Self {
            resources,
            dsh_home,
            web_profile,
            managed_plugins_root,
            bundled_market_root,
            legacy_market_root,
            user_home,
            linker,
        }
    }

    /// 校验资源并准备 profile；任何中途错误都必须保持原状态。
    pub fn prepare(&self) -> Result<PluginTransaction, String> {
        self.repair_legacy_skin_patch()?;
        // Market 属于桌面运行时层，必须先于可回滚的四个托管插件持久化。
        self.ensure_builtin_market()?;
        let lock_path = self.resources.join("plugins.lock.json");
        let lock_bytes = fs::read(&lock_path)
            .map_err(|error| format!("failed to read {}: {error}", lock_path.display()))?;
        let lock = PluginLock::parse(&lock_bytes)?;
        validate_plugin_tree(&self.resources.join("node_modules"), &lock)?;

        let digest = format!("{:x}", Sha256::digest(&lock_bytes));
        let store = self.managed_plugins_root.join(&digest[..16]);
        let store_modules = store.join("node_modules");
        ensure_plugin_store(&self.resources, &store, &lock, &lock_bytes)?;

        let state_path = self.dsh_home.join("desktop-managed/plugins-state.json");
        let state = read_json_or_default::<PluginInstallState>(&state_path)?;
        let profile_path = self.web_profile.join("package.json");
        let profile = read_profile(&profile_path)?;
        if self.fast_path_matches(&profile, &state, &lock, &store_modules, &digest)? {
            return Ok(PluginTransaction {
                should_seed_sidebar: false,
                state_path,
                next_state: state,
                snapshots: Vec::new(),
                link_changes: Vec::new(),
                linker: self.linker.clone(),
                finalized: false,
            });
        }
        let plan = plan_profile(profile.clone(), &state, &lock, &store_modules, &digest)?;
        let profile_changed = plan.profile != profile;
        if profile_changed && profile_path.is_file() {
            persist_profile_backup(&self.dsh_home, &profile_path)?;
        }
        let expected_profile_bytes = read_optional_bytes(&profile_path)?;
        let mut transaction = PluginTransaction {
            should_seed_sidebar: plan.next_state.managed.contains_key("dsh-better-sidebar")
                && !state.sidebar_defaults_seeded,
            state_path,
            next_state: plan.next_state,
            snapshots: Vec::new(),
            link_changes: Vec::new(),
            linker: self.linker.clone(),
            finalized: false,
        };

        let result = (|| {
            fs::create_dir_all(&self.web_profile).map_err(|error| {
                format!(
                    "failed to create web profile {}: {error}",
                    self.web_profile.display()
                )
            })?;
            let profile_modules = self.web_profile.join("node_modules");
            fs::create_dir_all(&profile_modules).map_err(|error| {
                format!(
                    "failed to create profile node_modules {}: {error}",
                    profile_modules.display()
                )
            })?;
            // 用户 patch 是可选文件；不预写 `[]`，避免第三方在其后追加顶层列表时产生非法 YAML。
            let workspace_path = self.web_profile.join("pnpm-workspace.yaml");
            if !workspace_path.exists() {
                transaction
                    .snapshots
                    .push(FileSnapshot::capture(&workspace_path)?);
                atomic_write(
                    &workspace_path,
                    b"packages:\n  - .\n\nnodeLinker: hoisted\nautoInstallPeers: false\n",
                )?;
            }

            for package in &plan.removed_packages {
                let Some(previous) = state.managed.get(package) else {
                    continue;
                };
                let link = profile_modules.join(package_relative_path(package)?);
                let current_target = self.linker.target(&link)?;
                if current_target.as_ref().is_some_and(|target| {
                    normalized_path(target) == normalized_path(Path::new(&previous.link_target))
                }) {
                    self.linker.remove(&link)?;
                    transaction.link_changes.push(LinkChange {
                        link,
                        previous_target: current_target,
                    });
                }
            }

            for package in &plan.managed_packages {
                let relative = package_relative_path(package)?;
                let link = profile_modules.join(&relative);
                let target = store_modules.join(&relative);
                let previous_target = self.linker.target(&link)?;
                if previous_target
                    .as_ref()
                    .is_some_and(|value| value == &target)
                {
                    continue;
                }
                if link.exists() && previous_target.is_none() {
                    return Err(format!(
                        "refusing to replace non-managed plugin directory: {}",
                        link.display()
                    ));
                }
                if previous_target.is_some() {
                    self.linker.remove(&link)?;
                }
                if let Some(parent) = link.parent() {
                    fs::create_dir_all(parent).map_err(|error| {
                        format!("failed to create {}: {error}", parent.display())
                    })?;
                }
                self.linker.create(&link, &target)?;
                transaction.link_changes.push(LinkChange {
                    link,
                    previous_target,
                });
            }

            if profile_changed {
                if read_optional_bytes(&profile_path)? != expected_profile_bytes {
                    return Err(format!(
                        "profile changed concurrently while plugins were being prepared: {}",
                        profile_path.display()
                    ));
                }
                transaction
                    .snapshots
                    .push(FileSnapshot::capture(&profile_path)?);
                atomic_write_json(&profile_path, &plan.profile)?;
            }

            for skill in &lock.skills {
                let source = store_modules
                    .join(package_relative_path(&skill.source_package)?)
                    .join(&skill.source_file);
                let content = fs::read(&source).map_err(|error| {
                    format!("failed to read managed skill {}: {error}", source.display())
                })?;
                let source_digest = sha256_hex(&content);
                let target = self
                    .dsh_home
                    .join("skills")
                    .join(&skill.name)
                    .join("SKILL.md");
                let current_digest = read_optional_bytes(&target)?.as_deref().map(sha256_hex);
                match plan_managed_skill(
                    skill,
                    state.managed_skills.get(&skill.name),
                    current_digest.as_deref(),
                    &source_digest,
                )? {
                    ManagedSkillAction::Write(next) => {
                        transaction.snapshots.push(FileSnapshot::capture(&target)?);
                        atomic_write(&target, &content)?;
                        transaction
                            .next_state
                            .managed_skills
                            .insert(skill.name.clone(), next);
                    }
                    ManagedSkillAction::RememberRemoved(next) => {
                        transaction
                            .next_state
                            .managed_skills
                            .insert(skill.name.clone(), next);
                    }
                    ManagedSkillAction::KeepManaged(next) => {
                        transaction
                            .next_state
                            .managed_skills
                            .insert(skill.name.clone(), next);
                    }
                    ManagedSkillAction::PreserveUnmanaged => {}
                }
            }

            // Hindsight 的配置目录不遵循 DSH_HOME；仅在桌面托管该插件且文件不存在时写入隐私关闭默认值。
            if transaction
                .next_state
                .managed
                .contains_key("@vectorize-io/hindsight-coding-agents")
            {
                let config_path = self.user_home.join(".hindsight/coding-agent.json");
                if !config_path.exists() {
                    transaction
                        .snapshots
                        .push(FileSnapshot::capture(&config_path)?);
                    atomic_write(
                        &config_path,
                        b"{\n  \"harnesses\": {\n    \"dsh\": {\n      \"optInOnly\": true,\n      \"optInPaths\": []\n    }\n  }\n}\n",
                    )?;
                }
            }
            Ok(())
        })();

        if let Err(error) = result {
            let rollback_error = transaction.rollback_internal().err();
            return Err(match rollback_error {
                Some(rollback) => format!("{error}; rollback failed: {rollback}"),
                None => error,
            });
        }
        Ok(transaction)
    }

    /// 校验健康 marker、profile、junction 与托管 Skill，命中时跳过重新规划和链接重建。
    fn fast_path_matches(
        &self,
        profile: &Value,
        state: &PluginInstallState,
        lock: &PluginLock,
        store_modules: &Path,
        digest: &str,
    ) -> Result<bool, String> {
        if state.lock_digest != digest
            || !state.sidebar_defaults_seeded
            || !self.web_profile.join("pnpm-workspace.yaml").is_file()
        {
            return Ok(false);
        }
        let Some(dependencies) = profile.get("dependencies").and_then(Value::as_object) else {
            return Ok(false);
        };
        let Some(bundle_values) = profile
            .pointer("/dsh/profile/bundles")
            .and_then(Value::as_array)
        else {
            return Ok(false);
        };
        let bundles = bundle_values
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();

        for (package, version, should_be_bundle) in lock
            .plugins
            .iter()
            .map(|plugin| (plugin.package.as_str(), plugin.version.as_str(), true))
            .chain(lock.transitive_packages.iter().map(|dependency| {
                (
                    dependency.package.as_str(),
                    dependency.version.as_str(),
                    false,
                )
            }))
        {
            let target = store_modules.join(package_relative_path(package)?);
            let target_text = normalized_path(&target);
            let expected_dependency = link_spec(&target_text);
            let Some(record) = state.managed.get(package) else {
                return Ok(false);
            };
            if record.version != version
                || record.link_target != target_text
                || dependencies.get(package).and_then(Value::as_str)
                    != Some(expected_dependency.as_str())
                || self.linker.target(
                    &self
                        .web_profile
                        .join("node_modules")
                        .join(package_relative_path(package)?),
                )? != Some(target)
                || (should_be_bundle && bundles.contains(&package)) != record.bundle_enabled
            {
                return Ok(false);
            }
        }

        let runtime_index = bundles
            .iter()
            .position(|bundle| *bundle == RUNTIME_SERVICES_BUNDLE);
        let market_index = bundles.iter().position(|bundle| *bundle == MARKET_BUNDLE);
        if !matches!(runtime_index.zip(market_index), Some((runtime, market)) if runtime + 1 == market)
        {
            return Ok(false);
        }
        for skill in &lock.skills {
            let Some(record) = state.managed_skills.get(&skill.name) else {
                return Ok(false);
            };
            if record.user_removed {
                continue;
            }
            let target = self
                .dsh_home
                .join("skills")
                .join(&skill.name)
                .join("SKILL.md");
            if read_optional_bytes(&target)?.as_deref().map(sha256_hex)
                != Some(record.content_sha256.clone())
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// 修复 preview.4 产生的 `[]` 加 Skin 管理块，修复先于插件事务并在 core retry 时保留。
    pub fn repair_legacy_skin_patch(&self) -> Result<bool, String> {
        let path = self.dsh_home.join("cordis.patch.yml");
        let Some(content) = read_optional_bytes(&path)? else {
            return Ok(false);
        };
        let Some(repaired) = repair_legacy_skin_patch_content(&content)? else {
            return Ok(false);
        };
        atomic_write(&path, &repaired)?;
        Ok(true)
    }

    /// 将内置 Market 非破坏性写入 web profile，并确保干净 profile 能解析运行时包。
    fn ensure_builtin_market(&self) -> Result<(), String> {
        self.remove_legacy_market_runtime_alias()?;
        let profile_path = self.web_profile.join("package.json");
        let mut profile = read_profile(&profile_path)?;
        let original = profile.clone();
        insert_builtin_market(&mut profile)?;
        let root = profile
            .as_object_mut()
            .ok_or_else(|| "profile package.json root must be an object".to_owned())?;
        let dependencies = object_field(root, "dependencies")?;
        let target = normalized_path(&self.bundled_market_root);
        let expected_dependency = link_spec(&target);
        let current_dependency = dependencies.get(MARKET_BUNDLE).and_then(Value::as_str);
        let desktop_managed = match current_dependency {
            None => {
                dependencies.insert(
                    MARKET_BUNDLE.to_owned(),
                    Value::String(expected_dependency.clone()),
                );
                true
            }
            Some(current) => current == expected_dependency,
        };

        let link = self.web_profile.join("node_modules").join(MARKET_BUNDLE);
        let previous_target = self.linker.target(&link)?;
        let expected_target = fs::canonicalize(&self.bundled_market_root)
            .unwrap_or_else(|_| self.bundled_market_root.clone());
        let points_to_bundled_market = previous_target
            .as_ref()
            .is_some_and(|current| normalized_path(current) == normalized_path(&expected_target));

        if desktop_managed {
            if !self.bundled_market_root.join("package.json").is_file() {
                return Err(format!(
                    "bundled Market is incomplete: {}",
                    self.bundled_market_root.display()
                ));
            }
            fs::create_dir_all(link.parent().expect("Market link always has a parent"))
                .map_err(|error| format!("failed to create Market link parent: {error}"))?;
            if link.exists() && previous_target.is_none() {
                return Err(format!(
                    "refusing to replace non-managed Market directory: {}",
                    link.display()
                ));
            }
            if !points_to_bundled_market {
                if previous_target.is_some() {
                    self.linker.remove(&link)?;
                }
                if let Err(error) = self.linker.create(&link, &self.bundled_market_root) {
                    if let Some(previous) = &previous_target {
                        let _ = self.linker.create(&link, previous);
                    }
                    return Err(error);
                }
            }
        } else if points_to_bundled_market {
            // 用户依赖优先；移除桌面端链接，交由 DSH 的包管理流程解析用户版本。
            self.linker.remove(&link)?;
        }

        if profile == original {
            return Ok(());
        }
        if profile_path.is_file() {
            persist_profile_backup(&self.dsh_home, &profile_path)?;
        }
        if let Err(error) = atomic_write_json(&profile_path, &profile) {
            let current_target = self.linker.target(&link)?;
            if current_target != previous_target {
                if current_target.is_some() {
                    let _ = self.linker.remove(&link);
                }
                if let Some(previous) = previous_target {
                    let _ = self.linker.create(&link, &previous);
                }
            }
            return Err(error);
        }
        Ok(())
    }

    /// 只清理目标精确指向旧桌面副本的 Market alias，普通目录与用户链接保持不变。
    fn remove_legacy_market_runtime_alias(&self) -> Result<(), String> {
        let alias = self
            .dsh_home
            .join("profiles/node_modules")
            .join(MARKET_RUNTIME_ALIAS);
        let expected = fs::canonicalize(&self.legacy_market_root)
            .unwrap_or_else(|_| self.legacy_market_root.clone());
        let current = self.linker.target(&alias)?;
        if current
            .as_ref()
            .is_some_and(|target| normalized_path(target) == normalized_path(&expected))
        {
            self.linker.remove(&alias)?;
        }
        Ok(())
    }
}

impl PluginTransaction {
    /// 返回首次托管 Better Sidebar 是否需要写入安全默认设置。
    pub fn should_seed_sidebar(&self) -> bool {
        self.should_seed_sidebar
    }

    /// Better Sidebar 安全设置写入成功后更新待提交 marker。
    pub fn mark_sidebar_seeded(&mut self) {
        self.next_state.sidebar_defaults_seeded = true;
    }

    /// Host 和设置初始化成功后持久化桌面 marker。
    pub fn commit(mut self) -> Result<(), String> {
        atomic_write_json_if_changed(&self.state_path, &self.next_state)?;
        self.finalized = true;
        Ok(())
    }

    /// 恢复本次写入前的 profile、marker 和目录链接。
    pub fn rollback(mut self) -> Result<(), String> {
        self.rollback_internal()
    }

    /// 执行实际恢复并聚合首个错误，供显式回滚和 `Drop` 共用。
    fn rollback_internal(&mut self) -> Result<(), String> {
        if self.finalized {
            return Ok(());
        }
        let mut first_error = None;
        for change in self.link_changes.iter().rev() {
            if let Err(error) = self.linker.remove(&change.link) {
                first_error.get_or_insert(error);
                continue;
            }
            if let Some(previous) = &change.previous_target {
                if let Err(error) = self.linker.create(&change.link, previous) {
                    first_error.get_or_insert(error);
                }
            }
        }
        for snapshot in self.snapshots.iter().rev() {
            if let Err(error) = snapshot.restore() {
                first_error.get_or_insert(error);
            }
        }
        self.finalized = true;
        first_error.map_or(Ok(()), Err)
    }
}

impl Drop for PluginTransaction {
    /// 未提交事务离开作用域时自动恢复，避免错误分支遗漏清理。
    fn drop(&mut self) {
        let _ = self.rollback_internal();
    }
}

impl FileSnapshot {
    /// 捕获文件当前内容，不存在时记录为空状态。
    fn capture(path: &Path) -> Result<Self, String> {
        let original = match fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(format!("failed to snapshot {}: {error}", path.display())),
        };
        Ok(Self {
            path: path.to_owned(),
            original,
        })
    }

    /// 将文件恢复到捕获时的字节或不存在状态。
    fn restore(&self) -> Result<(), String> {
        if let Some(bytes) = &self.original {
            atomic_write(&self.path, bytes)
        } else if self.path.exists() {
            fs::remove_file(&self.path)
                .map_err(|error| format!("failed to remove {}: {error}", self.path.display()))
        } else {
            Ok(())
        }
    }
}

impl DirectoryLinker for SystemDirectoryLinker {
    /// 只把 reparse point/symlink 识别为受控链接，普通目录永不覆盖。
    fn target(&self, link: &Path) -> Result<Option<PathBuf>, String> {
        let metadata = match fs::symlink_metadata(link) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("failed to inspect {}: {error}", link.display())),
        };
        if !is_directory_link(&metadata) {
            return Ok(None);
        }
        match fs::canonicalize(link) {
            Ok(target) => Ok(Some(target)),
            Err(canonical_error) => fs::read_link(link).map(Some).map_err(|read_error| {
                format!(
                    "failed to resolve {}: {canonical_error}; read link failed: {read_error}",
                    link.display()
                )
            }),
        }
    }

    /// Windows 使用 junction；其他平台仅供开发测试使用目录 symlink。
    fn create(&self, link: &Path, target: &Path) -> Result<(), String> {
        create_directory_link(&self.node, link, target)
    }

    /// 删除链接入口，不递归触碰其目标目录。
    fn remove(&self, link: &Path) -> Result<(), String> {
        if !link.exists() {
            return Ok(());
        }
        fs::remove_dir(link)
            .map_err(|error| format!("failed to remove plugin link {}: {error}", link.display()))
    }
}

/// 校验每个锁定插件的入口、patch、许可证和本地资产均已随包交付。
fn validate_plugin_tree(node_modules: &Path, lock: &PluginLock) -> Result<(), String> {
    for plugin in &lock.plugins {
        let package_root = node_modules.join(package_relative_path(&plugin.package)?);
        if !package_root.join("package.json").is_file() {
            return Err(format!(
                "plugin {} required file is missing: package.json",
                plugin.package
            ));
        }
        for required in &plugin.required_files {
            let required_path = package_root.join(required);
            if !required_path.is_file() && !required_path.is_dir() {
                return Err(format!(
                    "plugin {} required file is missing: {required}",
                    plugin.package
                ));
            }
        }
    }
    for dependency in &lock.transitive_packages {
        let package_root = node_modules.join(package_relative_path(&dependency.package)?);
        if !package_root.join("package.json").is_file() {
            return Err(format!(
                "managed dependency {} required file is missing: package.json",
                dependency.package
            ));
        }
        for required in &dependency.required_files {
            let required_path = package_root.join(required);
            if !required_path.is_file() && !required_path.is_dir() {
                return Err(format!(
                    "managed dependency {} required file is missing: {required}",
                    dependency.package
                ));
            }
        }
    }
    for skill in &lock.skills {
        let source = node_modules
            .join(package_relative_path(&skill.source_package)?)
            .join(&skill.source_file);
        let bytes = fs::read(&source)
            .map_err(|error| format!("managed skill {} source is missing: {error}", skill.name))?;
        let actual = sha256_hex(&bytes);
        if actual != skill.sha256 {
            return Err(format!(
                "managed skill {} SHA-256 mismatch: expected {}, got {actual}",
                skill.name, skill.sha256
            ));
        }
    }
    Ok(())
}

/// 将构建期资源复制到短摘要目录，已有完整缓存只做复核。
fn ensure_plugin_store(
    resources: &Path,
    store: &Path,
    lock: &PluginLock,
    lock_bytes: &[u8],
) -> Result<(), String> {
    let store_modules = store.join("node_modules");
    if validate_plugin_tree(&store_modules, lock).is_ok() {
        return Ok(());
    }
    if store.exists() {
        fs::remove_dir_all(store)
            .map_err(|error| format!("failed to remove invalid plugin cache: {error}"))?;
    }
    let parent = store
        .parent()
        .ok_or_else(|| "managed plugin store has no parent".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create plugin cache parent: {error}"))?;
    let staging = parent.join(format!(".staging-{}", std::process::id()));
    if staging.exists() {
        fs::remove_dir_all(&staging)
            .map_err(|error| format!("failed to clear plugin staging directory: {error}"))?;
    }
    let result = (|| {
        let source_modules = resources.join("node_modules");
        let staging_modules = staging.join("node_modules");
        copy_physical_tree(&source_modules, &staging_modules)?;
        // 源树已完整拒绝链接，Windows robocopy 同时使用 /XJ；目标只需按 lock 复核必要文件。
        atomic_write(&staging.join("plugins.lock.json"), lock_bytes)?;
        validate_plugin_tree(&staging_modules, lock)?;
        fs::rename(&staging, store).map_err(|error| {
            format!(
                "failed to activate plugin cache {}: {error}",
                store.display()
            )
        })
    })();
    if result.is_err() && staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

/// 递归复制普通文件和目录；Windows 使用多线程系统复制缩短首次启动时间。
fn copy_physical_tree(source: &Path, destination: &Path) -> Result<(), String> {
    validate_physical_tree(source)?;
    copy_validated_tree(source, destination)
}

/// 递归拒绝资源树中的链接、junction 和其他非常规文件。
fn validate_physical_tree(source: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("failed to inspect {}: {error}", source.display()))?;
    if is_directory_link(&metadata) || metadata.file_type().is_symlink() {
        return Err(format!(
            "plugin resources must not contain links: {}",
            source.display()
        ));
    }
    if metadata.is_file() {
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(format!("unsupported plugin resource: {}", source.display()));
    }
    for entry in fs::read_dir(source)
        .map_err(|error| format!("failed to read {}: {error}", source.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read plugin resource: {error}"))?;
        validate_physical_tree(&entry.path())?;
    }
    Ok(())
}

/// Windows 10/11 自带 robocopy；多线程复制 4,000 余个插件文件，避免阻塞启动数分钟。
#[cfg(windows)]
fn copy_validated_tree(source: &Path, destination: &Path) -> Result<(), String> {
    let mut command = Command::new("robocopy.exe");
    command
        .arg(source)
        .arg(destination)
        .args([
            "/E",
            "/COPY:DAT",
            "/DCOPY:DAT",
            "/R:1",
            "/W:1",
            "/MT:32",
            "/XJ",
            "/SL",
            "/NFL",
            "/NDL",
            "/NJH",
            "/NJS",
            "/NP",
        ])
        .creation_flags(CREATE_NO_WINDOW);
    let output = command
        .output()
        .map_err(|error| format!("failed to start robocopy.exe: {error}"))?;
    let exit_code = output.status.code().unwrap_or(i32::MAX);
    if exit_code > 7 {
        let detail = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        return Err(format!("robocopy failed with code {exit_code}: {detail}"));
    }
    Ok(())
}

/// 非 Windows 环境使用标准库逐文件复制，仅用于开发与单元测试。
#[cfg(not(windows))]
fn copy_validated_tree(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("failed to inspect {}: {error}", source.display()))?;
    if metadata.is_file() {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        }
        fs::copy(source, destination).map_err(|error| {
            format!(
                "failed to copy {} to {}: {error}",
                source.display(),
                destination.display()
            )
        })?;
        return Ok(());
    }
    fs::create_dir_all(destination)
        .map_err(|error| format!("failed to create {}: {error}", destination.display()))?;
    for entry in fs::read_dir(source)
        .map_err(|error| format!("failed to read {}: {error}", source.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read plugin resource: {error}"))?;
        copy_validated_tree(&entry.path(), &destination.join(entry.file_name()))?;
    }
    Ok(())
}

/// 读取 JSON marker；文件不存在时使用默认值，损坏时拒绝覆盖。
fn read_json_or_default<T>(path: &Path) -> Result<T, String>
where
    T: for<'de> Deserialize<'de> + Default,
{
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| format!("invalid JSON in {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(format!("failed to read {}: {error}", path.display())),
    }
}

/// 读取 web profile；缺失时从空对象初始化，错误结构留给迁移层拒绝。
fn read_profile(path: &Path) -> Result<Value, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| format!("invalid profile JSON in {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(Value::Object(Default::default()))
        }
        Err(error) => Err(format!("failed to read {}: {error}", path.display())),
    }
}

/// 读取可选文件的原始字节，用于并发写入保护。
fn read_optional_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("failed to read {}: {error}", path.display())),
    }
}

/// 识别 preview.4 的非法 Skin patch 形态并只移除首行空数组，后续管理块逐字保留。
fn repair_legacy_skin_patch_content(content: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let text = std::str::from_utf8(content)
        .map_err(|error| format!("skin patch is not valid UTF-8: {error}"))?;
    let first_line_end = text.find('\n').unwrap_or(text.len());
    if text[..first_line_end].trim_end_matches('\r') != "[]" {
        return Ok(None);
    }
    let remainder = text[first_line_end..].trim_start_matches(['\r', '\n']);
    let has_skin_marker = remainder.contains("# --- dsh-skin managed")
        || remainder
            .lines()
            .any(|line| line.trim_start().starts_with("- id: ui-skin-"));
    if !has_skin_marker {
        return Ok(None);
    }
    let mut repaired = remainder.as_bytes().to_vec();
    if !repaired.ends_with(b"\n") {
        repaired.push(b'\n');
    }
    Ok(Some(repaired))
}

/// 在修改现有 profile 前保留带时间戳的原始文件，供人工审计和恢复。
fn persist_profile_backup(dsh_home: &Path, profile_path: &Path) -> Result<(), String> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("system clock is before Unix epoch: {error}"))?
        .as_millis();
    let backup = dsh_home.join(format!(
        "desktop-managed/backups/{timestamp}-{}-web-package.json",
        std::process::id()
    ));
    let parent = backup
        .parent()
        .ok_or_else(|| "plugin backup path has no parent".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create plugin backup directory: {error}"))?;
    fs::copy(profile_path, &backup).map_err(|error| {
        format!(
            "failed to back up profile {} to {}: {error}",
            profile_path.display(),
            backup.display()
        )
    })?;
    Ok(())
}

/// 以格式化 JSON 写文件，便于用户审计 profile 和 marker。
fn atomic_write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("failed to encode {}: {error}", path.display()))?;
    bytes.push(b'\n');
    atomic_write(path, &bytes)
}

/// 仅在格式化 JSON 字节变化时执行原子替换，保持健康启动的文件 mtime 稳定。
fn atomic_write_json_if_changed(path: &Path, value: &impl Serialize) -> Result<bool, String> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("failed to encode {}: {error}", path.display()))?;
    bytes.push(b'\n');
    if read_optional_bytes(path)?.as_deref() == Some(bytes.as_slice()) {
        return Ok(false);
    }
    atomic_write(path, &bytes)?;
    Ok(true)
}

/// 先写同目录临时文件，再替换目标，防止进程中断留下半个 JSON。
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("invalid file name: {}", path.display()))?;
    let temporary = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
    fs::write(&temporary, bytes)
        .map_err(|error| format!("failed to write {}: {error}", temporary.display()))?;
    let result = replace_file(&temporary, path)
        .map_err(|error| format!("failed to activate {}: {error}", path.display()));
    if result.is_err() && temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(windows)]
/// 通过 Win32 原子替换同目录目标，避免删除与 rename 之间出现空窗。
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::iter;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "Kernel32")]
    extern "system" {
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    let source_wide = source
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    let destination_wide = destination
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: 两个 UTF-16 缓冲区都以 NUL 结尾，并在系统调用返回前保持有效。
    let result = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            destination_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
/// Unix rename 原生支持同文件系统内原子替换。
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
/// Windows junction 与 symlink 都带 reparse-point 属性。
fn is_directory_link(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
/// 非 Windows 开发环境用 symlink 类型识别目录链接。
fn is_directory_link(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
/// 通过 `mklink /J` 创建普通用户可用的 directory junction。
fn create_directory_link(node: &Path, link: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let status = Command::new(node)
        .arg("-e")
        .arg("require('node:fs').symlinkSync(process.argv[1], process.argv[2], 'junction')")
        .arg(target)
        .arg(link)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|error| format!("failed to create junction {}: {error}", link.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Node junction creation failed for {} with status {status}",
            link.display()
        ))
    }
}

#[cfg(not(windows))]
/// 非 Windows 开发环境创建目录 symlink，仅用于本地验证。
fn create_directory_link(_node: &Path, link: &Path, target: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(target, link)
        .map_err(|error| format!("failed to link {}: {error}", link.display()))
}

impl PluginLock {
    /// 解析并校验插件锁文件，拒绝未知 schema 和重复身份。
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let lock: Self = serde_json::from_slice(bytes)
            .map_err(|error| format!("invalid plugin lock JSON: {error}"))?;
        if lock.schema_version != SUPPORTED_SCHEMA_VERSION {
            return Err(format!(
                "unsupported plugin lock schema: {}",
                lock.schema_version
            ));
        }

        let mut packages = BTreeSet::new();
        let mut bundle_ids = BTreeSet::new();
        for plugin in &lock.plugins {
            validate_package_name(&plugin.package)?;
            if plugin.version.trim().is_empty() {
                return Err(format!("plugin {} has an empty version", plugin.package));
            }
            if plugin.bundle_id.trim().is_empty() {
                return Err(format!("plugin {} has an empty bundle id", plugin.package));
            }
            if plugin.license.trim().is_empty() {
                return Err(format!("plugin {} has an empty license", plugin.package));
            }
            validate_plugin_source(&plugin.package, &plugin.source)?;
            if !packages.insert(plugin.package.clone()) {
                return Err(format!(
                    "duplicate package in plugin lock: {}",
                    plugin.package
                ));
            }
            if !bundle_ids.insert(plugin.bundle_id.clone()) {
                return Err(format!(
                    "duplicate bundle id in plugin lock: {}",
                    plugin.bundle_id
                ));
            }
            for required in &plugin.required_files {
                validate_relative_path(required).map_err(|error| {
                    format!(
                        "plugin {} required file {required:?}: {error}",
                        plugin.package
                    )
                })?;
            }
        }
        for dependency in &lock.transitive_packages {
            validate_package_name(&dependency.package)?;
            if dependency.version.trim().is_empty() || dependency.license.trim().is_empty() {
                return Err(format!(
                    "managed dependency {} has incomplete metadata",
                    dependency.package
                ));
            }
            validate_npm_integrity(&dependency.package, &dependency.integrity)?;
            if !packages.insert(dependency.package.clone()) {
                return Err(format!(
                    "duplicate package in plugin lock: {}",
                    dependency.package
                ));
            }
            for required in &dependency.required_files {
                validate_relative_path(required).map_err(|error| {
                    format!(
                        "managed dependency {} required file {required:?}: {error}",
                        dependency.package
                    )
                })?;
            }
        }
        let plugin_packages = lock
            .plugins
            .iter()
            .map(|plugin| plugin.package.as_str())
            .collect::<BTreeSet<_>>();
        let mut skill_names = BTreeSet::new();
        for skill in &lock.skills {
            validate_package_name(&skill.source_package)?;
            validate_relative_path(&skill.source_file)?;
            validate_package_name(&skill.name)?;
            if skill.name.starts_with('@') || skill.name.contains('/') || skill.name.contains('\\')
            {
                return Err(format!(
                    "managed skill {} must use one directory name",
                    skill.name
                ));
            }
            if skill.version.trim().is_empty() || !is_lower_hex(&skill.sha256, 64) {
                return Err(format!(
                    "managed skill {} has incomplete metadata",
                    skill.name
                ));
            }
            if !plugin_packages.contains(skill.source_package.as_str()) {
                return Err(format!(
                    "managed skill {} references an unlocked source package",
                    skill.name
                ));
            }
            if !skill_names.insert(skill.name.clone()) {
                return Err(format!("duplicate managed skill: {}", skill.name));
            }
        }
        Ok(lock)
    }
}

/// 根据 marker 和当前文件摘要决定是否写入、让渡或记住用户删除。
fn plan_managed_skill(
    skill: &ManagedSkill,
    previous: Option<&ManagedSkillState>,
    current_sha256: Option<&str>,
    source_sha256: &str,
) -> Result<ManagedSkillAction, String> {
    if source_sha256 != skill.sha256 {
        return Err(format!(
            "managed skill {} source digest is invalid",
            skill.name
        ));
    }
    let next = |user_removed| ManagedSkillState {
        version: skill.version.clone(),
        content_sha256: skill.sha256.clone(),
        user_removed,
    };
    match (previous, current_sha256) {
        (None, None) => Ok(ManagedSkillAction::Write(next(false))),
        (None, Some(_)) => Ok(ManagedSkillAction::PreserveUnmanaged),
        (Some(_), None) => Ok(ManagedSkillAction::RememberRemoved(next(true))),
        (Some(record), Some(_)) if record.user_removed => Ok(ManagedSkillAction::PreserveUnmanaged),
        (Some(record), Some(current))
            if current == record.content_sha256 && current == skill.sha256 =>
        {
            Ok(ManagedSkillAction::KeepManaged(next(false)))
        }
        (Some(record), Some(current)) if current == record.content_sha256 => {
            Ok(ManagedSkillAction::Write(next(false)))
        }
        (Some(_), Some(_)) => Ok(ManagedSkillAction::PreserveUnmanaged),
    }
}

/// 根据当前 profile 与桌面 marker 计算非破坏性的下一状态。
fn plan_profile(
    mut profile: Value,
    state: &PluginInstallState,
    lock: &PluginLock,
    store_node_modules: &Path,
    lock_digest: &str,
) -> Result<ProfilePlan, String> {
    let root = profile
        .as_object_mut()
        .ok_or_else(|| "profile package.json root must be an object".to_owned())?;
    root.entry("name")
        .or_insert_with(|| Value::String("dsh-profile-web".to_owned()));
    root.entry("private").or_insert(Value::Bool(true));

    let mut dependency_values = object_field(root, "dependencies")?.clone();
    let bundles = profile_bundles(root)?;
    let mut current_bundles = bundles
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if current_bundles.is_empty() {
        current_bundles.extend([BASE_BUNDLE.to_owned(), WEB_APP_BUNDLE.to_owned()]);
    }
    current_bundles.retain(|bundle| bundle != LEGACY_SIDE_PANEL);
    insert_market_bundle(&mut current_bundles);

    // 只迁移仍与桌面 marker 匹配的旧 Skills/MCP 依赖；用户替换的来源保持原样。
    let mut removed_packages = Vec::new();
    let legacy_skills_enabled = state
        .managed
        .get(LEGACY_SKILLS_MCP_BUNDLE)
        .and_then(|record| {
            let dependency = dependency_values
                .get(LEGACY_SKILLS_MCP_BUNDLE)
                .and_then(Value::as_str)?;
            (dependency == link_spec(&record.link_target)).then(|| {
                let enabled = current_bundles
                    .iter()
                    .any(|bundle| bundle == LEGACY_SKILLS_MCP_BUNDLE);
                dependency_values.remove(LEGACY_SKILLS_MCP_BUNDLE);
                current_bundles.retain(|bundle| bundle != LEGACY_SKILLS_MCP_BUNDLE);
                removed_packages.push(LEGACY_SKILLS_MCP_BUNDLE.to_owned());
                enabled
            })
        });

    let mut next_state = PluginInstallState {
        schema_version: SUPPORTED_SCHEMA_VERSION,
        lock_digest: lock_digest.to_owned(),
        managed: BTreeMap::new(),
        managed_skills: BTreeMap::new(),
        sidebar_defaults_seeded: state.sidebar_defaults_seeded,
    };
    let mut managed_packages = Vec::new();

    for plugin in &lock.plugins {
        let target = store_node_modules.join(package_relative_path(&plugin.package)?);
        let target_text = normalized_path(&target);
        let previous = state.managed.get(&plugin.package);
        let current_dependency = dependency_values
            .get(&plugin.package)
            .and_then(Value::as_str);
        let was_owned = previous
            .zip(current_dependency)
            .is_some_and(|(record, dependency)| dependency == link_spec(&record.link_target));
        let matches_current_store =
            current_dependency.is_some_and(|dependency| dependency == link_spec(&target_text));

        // marker 已存在但依赖被用户删除，或依赖被替换为其他来源，都视为用户接管。
        if current_dependency.is_some() && previous.is_none() && !matches_current_store
            || current_dependency.is_some() && previous.is_some() && !was_owned
            || current_dependency.is_none() && previous.is_some()
        {
            continue;
        }

        dependency_values.insert(
            plugin.package.clone(),
            Value::String(link_spec(&target_text)),
        );
        let bundle_enabled = if [RUNTIME_SERVICES_BUNDLE, DESKTOP_SETTINGS_BUNDLE]
            .contains(&plugin.package.as_str())
        {
            true
        } else if plugin.package == SKILLS_MCP_BUNDLE && legacy_skills_enabled.is_some() {
            legacy_skills_enabled.unwrap_or(true)
        } else {
            previous
                .map(|_| {
                    current_bundles
                        .iter()
                        .any(|bundle| bundle == &plugin.package)
                })
                .unwrap_or(true)
        };
        current_bundles.retain(|bundle| bundle != &plugin.package);
        next_state.managed.insert(
            plugin.package.clone(),
            ManagedPluginState {
                version: plugin.version.clone(),
                link_target: target_text,
                bundle_enabled,
            },
        );
        managed_packages.push(plugin.package.clone());
    }

    // Skin Center 等伴随依赖需要 profile-local junction，但不能作为独立 bundle 激活。
    for dependency in &lock.transitive_packages {
        let target = store_node_modules.join(package_relative_path(&dependency.package)?);
        let target_text = normalized_path(&target);
        let previous = state.managed.get(&dependency.package);
        let current_dependency = dependency_values
            .get(&dependency.package)
            .and_then(Value::as_str);
        let was_owned = previous
            .zip(current_dependency)
            .is_some_and(|(record, current)| current == link_spec(&record.link_target));
        let matches_current_store =
            current_dependency.is_some_and(|current| current == link_spec(&target_text));
        if current_dependency.is_some() && previous.is_none() && !matches_current_store
            || current_dependency.is_some() && previous.is_some() && !was_owned
            || current_dependency.is_none() && previous.is_some()
        {
            continue;
        }
        dependency_values.insert(
            dependency.package.clone(),
            Value::String(link_spec(&target_text)),
        );
        // DSH CLI 会把 profile 中已有的所有直接依赖顺带写入 bundles。伴随依赖只能用于
        // Node 解析，若保留为顶层 bundle，会与聚合包中的同一 loader entry 重复注册。
        current_bundles.retain(|bundle| bundle != &dependency.package);
        next_state.managed.insert(
            dependency.package.clone(),
            ManagedPluginState {
                version: dependency.version.clone(),
                link_target: target_text,
                bundle_enabled: false,
            },
        );
        managed_packages.push(dependency.package.clone());
    }

    let mut managed_bundles = lock
        .plugins
        .iter()
        .filter(|plugin| {
            next_state
                .managed
                .get(&plugin.package)
                .is_some_and(|record| record.bundle_enabled)
        })
        .map(|plugin| plugin.package.clone())
        .collect::<Vec<_>>();
    let runtime_services = managed_bundles
        .iter()
        .position(|bundle| bundle == RUNTIME_SERVICES_BUNDLE)
        .map(|index| managed_bundles.remove(index));
    let market_index = current_bundles
        .iter()
        .position(|bundle| bundle == MARKET_BUNDLE)
        .unwrap_or(0);
    if let Some(runtime_services) = runtime_services {
        current_bundles.insert(market_index, runtime_services);
    }
    let managed_insertion = current_bundles
        .iter()
        .position(|bundle| bundle == MARKET_BUNDLE)
        .map_or(0, |index| index + 1);
    current_bundles.splice(managed_insertion..managed_insertion, managed_bundles);
    deduplicate(&mut current_bundles);

    *object_field(root, "dependencies")? = dependency_values;
    *profile_bundles(root)? = current_bundles.into_iter().map(Value::String).collect();
    Ok(ProfilePlan {
        profile,
        next_state,
        managed_packages,
        removed_packages,
    })
}

/// 将内置 Market 写入完整 profile，供托管插件校验前的独立持久化步骤使用。
fn insert_builtin_market(profile: &mut Value) -> Result<(), String> {
    let root = profile
        .as_object_mut()
        .ok_or_else(|| "profile package.json root must be an object".to_owned())?;
    root.entry("name")
        .or_insert_with(|| Value::String("dsh-profile-web".to_owned()));
    root.entry("private").or_insert(Value::Bool(true));
    let bundles = profile_bundles(root)?;
    let mut current = bundles
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if current.is_empty() {
        current.extend([BASE_BUNDLE.to_owned(), WEB_APP_BUNDLE.to_owned()]);
    }
    insert_market_bundle(&mut current);
    *bundles = current.into_iter().map(Value::String).collect();
    Ok(())
}

/// 将 Market 固定在官方 bundle 和桌面核心服务之后，统一准备阶段的稳定顺序。
fn insert_market_bundle(bundles: &mut Vec<String>) {
    bundles.retain(|bundle| bundle != MARKET_BUNDLE);
    deduplicate(bundles);
    let official_prefix = bundles
        .iter()
        .take_while(|bundle| bundle.starts_with("@deepseek-ai/"))
        .count();
    let insertion = if bundles
        .get(official_prefix)
        .is_some_and(|bundle| bundle == RUNTIME_SERVICES_BUNDLE)
    {
        official_prefix + 1
    } else {
        official_prefix
    };
    bundles.insert(insertion, MARKET_BUNDLE.to_owned());
}

/// 校验 npm 包名，避免锁文件内容逃逸出 `node_modules`。
fn validate_package_name(package: &str) -> Result<(), String> {
    let valid_segment = |segment: &str| {
        !segment.is_empty()
            && segment != "."
            && segment != ".."
            && segment
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || ".-_".contains(character))
    };
    let valid = if let Some(scoped) = package.strip_prefix('@') {
        let mut segments = scoped.split('/');
        matches!((segments.next(), segments.next(), segments.next()), (Some(scope), Some(name), None) if valid_segment(scope) && valid_segment(name))
    } else {
        !package.contains('/') && valid_segment(package)
    };
    valid
        .then_some(())
        .ok_or_else(|| format!("invalid plugin package name: {package}"))
}

/// 校验来源字段采用预期算法且 commit/hash 不是截断值。
fn validate_plugin_source(package: &str, source: &PluginSource) -> Result<(), String> {
    match source {
        PluginSource::Npm { integrity } => validate_npm_integrity(package, integrity),
        PluginSource::GithubTarball {
            url,
            commit,
            sha256,
        } => {
            if !url.starts_with("https://api.github.com/repos/") {
                return Err(format!("plugin {package} has an untrusted archive URL"));
            }
            if !is_lower_hex(commit, 40) {
                return Err(format!("plugin {package} has an invalid commit"));
            }
            if !is_lower_hex(sha256, 64) {
                return Err(format!("plugin {package} has an invalid SHA-256"));
            }
            Ok(())
        }
        PluginSource::GithubReleaseAsset {
            repository,
            tag,
            commit,
            url,
            sha256,
        } => {
            if repository.trim().is_empty()
                || tag.trim().is_empty()
                || !url.starts_with(&format!(
                    "https://github.com/{repository}/releases/download/{tag}/"
                ))
            {
                return Err(format!(
                    "plugin {package} has an untrusted release asset URL"
                ));
            }
            if !is_lower_hex(commit, 40) {
                return Err(format!("plugin {package} has an invalid release commit"));
            }
            if !is_lower_hex(sha256, 64) {
                return Err(format!("plugin {package} has an invalid SHA-256"));
            }
            Ok(())
        }
        PluginSource::Local { path } => validate_relative_path(path)
            .map_err(|error| format!("plugin {package} local source {path:?}: {error}")),
    }
}

/// 计算文件内容的规范小写 SHA-256。
fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// 校验 npm SRI 明确使用 SHA-512 且包含摘要正文。
fn validate_npm_integrity(package: &str, integrity: &str) -> Result<(), String> {
    let digest = integrity.strip_prefix("sha512-").unwrap_or_default();
    if digest.len() < 80
        || !digest
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || "+/=".contains(value))
    {
        return Err(format!("package {package} has an invalid npm integrity"));
    }
    Ok(())
}

/// 检查固定长度的小写十六进制摘要。
fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .chars()
            .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character))
}

/// 校验锁文件内路径只能指向插件包内部。
fn validate_relative_path(value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if value.trim().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("path must stay inside the plugin package".to_owned());
    }
    Ok(())
}

/// 返回 npm scope 对应的相对目录。
fn package_relative_path(package: &str) -> Result<std::path::PathBuf, String> {
    validate_package_name(package)?;
    Ok(package.split('/').collect())
}

/// 将路径转换成 pnpm/npm 可移植的 `link:` 表达形式。
fn normalized_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// 生成 profile dependency 使用的本地链接 spec。
fn link_spec(target: &str) -> String {
    format!("link:{}", target.replace('\\', "/"))
}

/// 获取或创建对象字段，并拒绝已有的错误类型。
fn object_field<'a>(
    root: &'a mut serde_json::Map<String, Value>,
    name: &str,
) -> Result<&'a mut serde_json::Map<String, Value>, String> {
    root.entry(name)
        .or_insert_with(|| Value::Object(Default::default()));
    root.get_mut(name)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| format!("profile field {name:?} must be an object"))
}

/// 获取 profile bundle 数组，并补齐缺失的中间对象。
fn profile_bundles(root: &mut serde_json::Map<String, Value>) -> Result<&mut Vec<Value>, String> {
    let dsh = object_field(root, "dsh")?;
    let profile = object_field(dsh, "profile")?;
    profile
        .entry("bundles")
        .or_insert_with(|| Value::Array(Vec::new()));
    profile
        .get_mut("bundles")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "profile field \"dsh.profile.bundles\" must be an array".to_owned())
}

/// 保持首次出现顺序并删除重复 bundle。
fn deduplicate(values: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use serde_json::{json, Value};

    use super::{
        copy_physical_tree, normalized_path, package_relative_path, plan_managed_skill,
        plan_profile, repair_legacy_skin_patch_content, sha256_hex, DirectoryLinker,
        ManagedPluginState, ManagedSkill, ManagedSkillAction, ManagedSkillState,
        PluginInstallState, PluginLock, PluginManager, BASE_BUNDLE, DESKTOP_SETTINGS_BUNDLE,
        LEGACY_SIDE_PANEL, LEGACY_SKILLS_MCP_BUNDLE, MARKET_BUNDLE, MARKET_RUNTIME_ALIAS,
        RUNTIME_SERVICES_BUNDLE, SKILLS_MCP_BUNDLE, WEB_APP_BUNDLE,
    };

    fn lock() -> PluginLock {
        PluginLock::parse(
            br#"{
              "schemaVersion": 1,
              "sharedPackages": ["@deepseek-ai", "react", "react-dom"],
              "plugins": [
                {"package":"@dsh-desktop/runtime-services","version":"0.1.0-preview.7","bundleId":"desktop-runtime-services","license":"MIT","source":{"type":"local","path":"desktop-plugins/runtime-services"},"requiredFiles":["lib/index.js"]},
                {"package":"dsh-at-file","version":"0.6.0","bundleId":"dsh-at-file","license":"MIT","source":{"type":"npm","integrity":"sha512-iKOgZ1auSGj2TyIjsS2nDqYiHrGWHUg08CxcIzgnkRjDyCjb/qjpt6W3cMLAj4KxTD2643+E7dg3nikClO0Esg=="},"requiredFiles":["lib/index.js"]},
                {"package":"@omdsh-dev/dsh-genui","version":"0.8.4","bundleId":"genui","license":"MIT","source":{"type":"npm","integrity":"sha512-iKOgZ1auSGj2TyIjsS2nDqYiHrGWHUg08CxcIzgnkRjDyCjb/qjpt6W3cMLAj4KxTD2643+E7dg3nikClO0Esg=="},"requiredFiles":["lib/index.js"]},
                {"package":"dsh-better-sidebar","version":"0.12.2","bundleId":"better-sidebar","license":"MIT","source":{"type":"npm","integrity":"sha512-iKOgZ1auSGj2TyIjsS2nDqYiHrGWHUg08CxcIzgnkRjDyCjb/qjpt6W3cMLAj4KxTD2643+E7dg3nikClO0Esg=="},"requiredFiles":["lib/index.js"]},
                {"package":"@dsh-desktop/theme-settings","version":"0.1.0","bundleId":"desktop-theme-settings","license":"MIT","source":{"type":"local","path":"desktop-plugins/theme-settings"},"requiredFiles":["lib/index.js","lib/client.js","cordis.patch.yml"]},
                {"package":"@linxin666/dsh-skins","version":"0.1.17","bundleId":"ui-skin-center","license":"Apache-2.0","source":{"type":"npm","integrity":"sha512-iKOgZ1auSGj2TyIjsS2nDqYiHrGWHUg08CxcIzgnkRjDyCjb/qjpt6W3cMLAj4KxTD2643+E7dg3nikClO0Esg=="},"requiredFiles":["cordis.patch.yml"]},
                {"package":"@vectorize-io/hindsight-coding-agents","version":"0.3.4","bundleId":"hindsight-coding-agents","license":"MIT","source":{"type":"npm","integrity":"sha512-iKOgZ1auSGj2TyIjsS2nDqYiHrGWHUg08CxcIzgnkRjDyCjb/qjpt6W3cMLAj4KxTD2643+E7dg3nikClO0Esg=="},"requiredFiles":["dist/dsh.js"]},
                {"package":"@liustack/modlens","version":"3.16.7","bundleId":"modlens","license":"MIT","source":{"type":"npm","integrity":"sha512-iKOgZ1auSGj2TyIjsS2nDqYiHrGWHUg08CxcIzgnkRjDyCjb/qjpt6W3cMLAj4KxTD2643+E7dg3nikClO0Esg=="},"requiredFiles":["dsh/index.js"]},
                {"package":"@cubee-slide/skills-mcp-manager","version":"0.2.3","bundleId":"skills-mcp-manager","license":"MIT","source":{"type":"npm","integrity":"sha512-JuPhoftrDPul29NcLac/BuB0JTArsTCOsTG8/nJnpRRjM03ADa2rDSuREV/HGGQdfs1JRTPHWz6h8mBNOmhWlA=="},"requiredFiles":["lib/index.js"]}
              ],
              "transitivePackages": [
                {"package":"@linxin666/dsh-client-ui-skin-center","version":"0.1.17","license":"Apache-2.0","integrity":"sha512-E3/sgA+igBiW/Hp1XPQwHB0BqzbokqOhb+U46hTJHGWGMbYtSQkCUZqIZ6ztHPyd3YtMMzY84LGS1SCYa57O8g==","requiredFiles":["lib/index.js","lib/client.js","cordis.patch.yml"]}
              ],
              "skills":[{"name":"genui","sourcePackage":"@omdsh-dev/dsh-genui","sourceFile":"SKILL.md","version":"0.8.4","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]
            }"#,
        )
        .unwrap()
    }

    fn bundles(profile: &Value) -> Vec<&str> {
        profile["dsh"]["profile"]["bundles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect()
    }

    #[test]
    fn lock_rejects_unknown_schema_and_duplicate_packages() {
        let schema = PluginLock::parse(br#"{"schemaVersion":2,"plugins":[]}"#).unwrap_err();
        assert!(schema.contains("schema"));

        let duplicate = PluginLock::parse(
            br#"{"schemaVersion":1,"plugins":[
              {"package":"same","version":"1","bundleId":"one","license":"MIT","source":{"type":"npm","integrity":"sha512-iKOgZ1auSGj2TyIjsS2nDqYiHrGWHUg08CxcIzgnkRjDyCjb/qjpt6W3cMLAj4KxTD2643+E7dg3nikClO0Esg=="}},
              {"package":"same","version":"2","bundleId":"two","license":"MIT","source":{"type":"npm","integrity":"sha512-iKOgZ1auSGj2TyIjsS2nDqYiHrGWHUg08CxcIzgnkRjDyCjb/qjpt6W3cMLAj4KxTD2643+E7dg3nikClO0Esg=="}}
            ]}"#,
        )
        .unwrap_err();
        assert!(duplicate.contains("duplicate package"));
    }

    #[test]
    fn old_plugin_state_without_managed_skills_remains_compatible() {
        let state: PluginInstallState = serde_json::from_slice(
            br#"{"schemaVersion":1,"lockDigest":"old","managed":{},"sidebarDefaultsSeeded":true}"#,
        )
        .unwrap();
        assert!(state.managed_skills.is_empty());
        assert!(state.sidebar_defaults_seeded);
    }

    #[test]
    fn lock_rejects_truncated_archive_hash() {
        let error = PluginLock::parse(
            br#"{"schemaVersion":1,"plugins":[{
              "package":"dsh-at-file","version":"0.6.0","bundleId":"dsh-at-file","license":"MIT",
              "source":{"type":"github-tarball","url":"https://api.github.com/repos/omdsh-dev/dsh-at-file/tarball/v0.6.0","commit":"a967aeb1df52b57609e6512b9b7bfd38b7baa092","sha256":"798825"}
            }]}"#,
        )
        .unwrap_err();
        assert!(error.contains("SHA-256"));
    }

    #[test]
    fn legacy_empty_array_before_skin_block_is_repaired_to_a_valid_top_level_list() {
        let broken = b"[]\r\n\r\n# --- dsh-skin managed (auto-generated; do not edit) ---\r\n- id: ui-skin-blue-fantasy\r\n  disabled: true\r\n";
        let repaired = repair_legacy_skin_patch_content(broken)
            .unwrap()
            .expect("known preview.4 shape must be repaired");
        let text = String::from_utf8(repaired).unwrap();
        assert!(text.starts_with("# --- dsh-skin managed"));
        assert!(text.contains("- id: ui-skin-blue-fantasy"));
        assert!(!text.starts_with("[]"));
        let yaml: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
        assert!(yaml.is_sequence());
    }

    #[test]
    fn unrelated_empty_array_patch_is_not_rewritten() {
        assert!(repair_legacy_skin_patch_content(b"[]\n# user content\n")
            .unwrap()
            .is_none());
    }

    #[test]
    fn clean_profile_receives_exact_managed_order_and_link_dependencies() {
        let plan = plan_profile(
            json!({}),
            &PluginInstallState::default(),
            &lock(),
            Path::new(r"C:\managed\node_modules"),
            "digest-a",
        )
        .unwrap();

        assert_eq!(
            bundles(&plan.profile),
            vec![
                BASE_BUNDLE,
                WEB_APP_BUNDLE,
                RUNTIME_SERVICES_BUNDLE,
                "dshmarket",
                "dsh-at-file",
                "@omdsh-dev/dsh-genui",
                "dsh-better-sidebar",
                "@dsh-desktop/theme-settings",
                "@linxin666/dsh-skins",
                "@vectorize-io/hindsight-coding-agents",
                "@liustack/modlens",
                SKILLS_MCP_BUNDLE
            ]
        );
        assert_eq!(plan.managed_packages.len(), 10);
        assert_eq!(plan.next_state.lock_digest, "digest-a");
        assert!(plan.profile["dependencies"]["dsh-at-file"]
            .as_str()
            .unwrap()
            .starts_with("link:"));
    }

    fn genui_skill() -> ManagedSkill {
        ManagedSkill {
            name: "genui".to_owned(),
            source_package: "@omdsh-dev/dsh-genui".to_owned(),
            source_file: "SKILL.md".to_owned(),
            version: "0.8.4".to_owned(),
            sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
        }
    }

    #[test]
    fn managed_skill_first_install_and_unmodified_upgrade_are_written() {
        let skill = genui_skill();
        assert!(matches!(
            plan_managed_skill(
                &skill,
                None,
                None,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            )
            .unwrap(),
            ManagedSkillAction::Write(_)
        ));
        let current = ManagedSkillState {
            version: "0.8.4".to_owned(),
            content_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_owned(),
            user_removed: false,
        };
        assert!(matches!(
            plan_managed_skill(
                &skill,
                Some(&current),
                Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .unwrap(),
            ManagedSkillAction::KeepManaged(_)
        ));
        let previous = ManagedSkillState {
            version: "0.8.3".to_owned(),
            content_sha256: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_owned(),
            user_removed: false,
        };
        assert!(matches!(
            plan_managed_skill(
                &skill,
                Some(&previous),
                Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .unwrap(),
            ManagedSkillAction::Write(_)
        ));
    }

    #[test]
    fn managed_skill_preserves_unmanaged_modified_and_deleted_files() {
        let skill = genui_skill();
        assert!(matches!(
            plan_managed_skill(
                &skill,
                None,
                Some("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"),
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .unwrap(),
            ManagedSkillAction::PreserveUnmanaged
        ));
        let previous = ManagedSkillState {
            version: "0.8.3".to_owned(),
            content_sha256: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_owned(),
            user_removed: false,
        };
        assert!(matches!(
            plan_managed_skill(
                &skill,
                Some(&previous),
                Some("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"),
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .unwrap(),
            ManagedSkillAction::PreserveUnmanaged
        ));
        assert!(matches!(
            plan_managed_skill(
                &skill,
                Some(&previous),
                None,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .unwrap(),
            ManagedSkillAction::RememberRemoved(_)
        ));
    }

    #[test]
    fn user_installed_plugin_wins_and_is_not_marked_as_managed() {
        let profile = json!({
          "dependencies": {"dsh-at-file": "https://example.test/user-plugin.tgz"},
          "dsh": {"profile": {"bundles": [BASE_BUNDLE, "dsh-at-file"]}}
        });
        let plan = plan_profile(
            profile,
            &PluginInstallState::default(),
            &lock(),
            Path::new(r"C:\managed\node_modules"),
            "digest-b",
        )
        .unwrap();

        assert_eq!(
            plan.profile["dependencies"]["dsh-at-file"],
            "https://example.test/user-plugin.tgz"
        );
        assert!(!plan.next_state.managed.contains_key("dsh-at-file"));
        assert!(!plan.managed_packages.contains(&"dsh-at-file".to_owned()));
    }

    #[test]
    fn interrupted_desktop_link_is_reclaimed_when_marker_is_missing() {
        let store = Path::new(r"C:\managed\node_modules");
        let profile = json!({
          "dependencies": {"dsh-at-file": "link:C:/managed/node_modules/dsh-at-file"},
          "dsh": {"profile": {"bundles": [BASE_BUNDLE, WEB_APP_BUNDLE, "dsh-at-file"]}}
        });
        let plan = plan_profile(
            profile,
            &PluginInstallState::default(),
            &lock(),
            store,
            "digest-recovery",
        )
        .unwrap();
        assert!(plan.next_state.managed.contains_key("dsh-at-file"));
        assert!(plan.managed_packages.contains(&"dsh-at-file".to_owned()));
    }

    #[test]
    fn managed_bundle_block_is_inserted_after_official_prefix_before_user_bundles() {
        let profile = json!({
          "dsh": {"profile": {"bundles": [BASE_BUNDLE, WEB_APP_BUNDLE, "user-bundle"]}}
        });
        let plan = plan_profile(
            profile,
            &PluginInstallState::default(),
            &lock(),
            Path::new(r"C:\managed\node_modules"),
            "digest-order",
        )
        .unwrap();
        assert_eq!(
            bundles(&plan.profile),
            vec![
                BASE_BUNDLE,
                WEB_APP_BUNDLE,
                RUNTIME_SERVICES_BUNDLE,
                "dshmarket",
                "dsh-at-file",
                "@omdsh-dev/dsh-genui",
                "dsh-better-sidebar",
                "@dsh-desktop/theme-settings",
                "@linxin666/dsh-skins",
                "@vectorize-io/hindsight-coding-agents",
                "@liustack/modlens",
                SKILLS_MCP_BUNDLE,
                "user-bundle"
            ]
        );
    }

    #[test]
    fn transitive_skin_center_is_removed_from_historical_top_level_bundles() {
        let profile = json!({
          "dependencies": {
            "@linxin666/dsh-client-ui-skin-center": "link:C:/managed/node_modules/@linxin666/dsh-client-ui-skin-center"
          },
          "dsh": {"profile": {"bundles": [
            BASE_BUNDLE,
            WEB_APP_BUNDLE,
            "@linxin666/dsh-skins",
            "@linxin666/dsh-client-ui-skin-center"
          ]}}
        });
        let plan = plan_profile(
            profile,
            &PluginInstallState::default(),
            &lock(),
            Path::new(r"C:\managed\node_modules"),
            "digest-transitive-bundle",
        )
        .unwrap();

        let active = bundles(&plan.profile);
        assert!(active.contains(&"@linxin666/dsh-skins"));
        assert!(!active.contains(&"@linxin666/dsh-client-ui-skin-center"));
        assert!(!plan.next_state.managed["@linxin666/dsh-client-ui-skin-center"].bundle_enabled);
    }

    #[test]
    fn bundled_market_is_active_without_replacing_user_dependency() {
        let profile = json!({
          "dependencies": {"dshmarket": "1.4.0"},
          "dsh": {"profile": {"bundles": [BASE_BUNDLE, WEB_APP_BUNDLE, "user-bundle"]}}
        });
        let plan = plan_profile(
            profile,
            &PluginInstallState::default(),
            &lock(),
            Path::new(r"C:\managed\node_modules"),
            "digest-market",
        )
        .unwrap();

        assert_eq!(plan.profile["dependencies"]["dshmarket"], "1.4.0");
        assert_eq!(
            bundles(&plan.profile),
            vec![
                BASE_BUNDLE,
                WEB_APP_BUNDLE,
                RUNTIME_SERVICES_BUNDLE,
                "dshmarket",
                "dsh-at-file",
                "@omdsh-dev/dsh-genui",
                "dsh-better-sidebar",
                "@dsh-desktop/theme-settings",
                "@linxin666/dsh-skins",
                "@vectorize-io/hindsight-coding-agents",
                "@liustack/modlens",
                SKILLS_MCP_BUNDLE,
                "user-bundle"
            ]
        );
    }

    #[test]
    fn removing_a_managed_bundle_is_preserved_as_user_disable() {
        let mut managed = BTreeMap::new();
        managed.insert(
            "dsh-at-file".to_owned(),
            ManagedPluginState {
                version: "0.5.1".to_owned(),
                link_target: r"C:\old\dsh-at-file".to_owned(),
                bundle_enabled: true,
            },
        );
        let state = PluginInstallState {
            schema_version: 1,
            lock_digest: "old".to_owned(),
            managed,
            managed_skills: BTreeMap::new(),
            sidebar_defaults_seeded: true,
        };
        let profile = json!({
          "dependencies": {"dsh-at-file": "link:C:/old/dsh-at-file"},
          "dsh": {"profile": {"bundles": [BASE_BUNDLE]}}
        });
        let plan = plan_profile(
            profile,
            &state,
            &lock(),
            Path::new(r"C:\managed\node_modules"),
            "digest-c",
        )
        .unwrap();

        assert!(!bundles(&plan.profile).contains(&"dsh-at-file"));
        assert!(!plan.next_state.managed["dsh-at-file"].bundle_enabled);
        assert_eq!(plan.next_state.managed["dsh-at-file"].version, "0.6.0");
    }

    #[test]
    fn newly_managed_plugin_disable_survives_upgrade_but_control_bundle_is_restored() {
        let store = Path::new(r"C:\managed\node_modules");
        let mut managed = BTreeMap::new();
        for package in [
            "@vectorize-io/hindsight-coding-agents",
            DESKTOP_SETTINGS_BUNDLE,
        ] {
            managed.insert(
                package.to_owned(),
                ManagedPluginState {
                    version: "old".to_owned(),
                    link_target: normalized_path(
                        &store.join(package_relative_path(package).unwrap()),
                    ),
                    bundle_enabled: true,
                },
            );
        }
        let state = PluginInstallState {
            schema_version: 1,
            lock_digest: "old".to_owned(),
            managed,
            managed_skills: BTreeMap::new(),
            sidebar_defaults_seeded: true,
        };
        let profile = json!({
          "dependencies": {
            "@vectorize-io/hindsight-coding-agents": "link:C:/managed/node_modules/@vectorize-io/hindsight-coding-agents",
            "@dsh-desktop/theme-settings": "link:C:/managed/node_modules/@dsh-desktop/theme-settings"
          },
          "dsh": {"profile": {"bundles": [BASE_BUNDLE, MARKET_BUNDLE]}}
        });
        let plan = plan_profile(profile, &state, &lock(), store, "new").unwrap();
        let active = bundles(&plan.profile);

        assert!(!active.contains(&"@vectorize-io/hindsight-coding-agents"));
        assert!(active.contains(&DESKTOP_SETTINGS_BUNDLE));
        assert!(!plan.next_state.managed["@vectorize-io/hindsight-coding-agents"].bundle_enabled);
        assert!(plan.next_state.managed[DESKTOP_SETTINGS_BUNDLE].bundle_enabled);
        assert!(active.contains(&RUNTIME_SERVICES_BUNDLE));
        assert!(plan.next_state.managed[RUNTIME_SERVICES_BUNDLE].bundle_enabled);
    }

    #[test]
    fn legacy_skills_mcp_disabled_state_migrates_to_desktop_package() {
        let store = Path::new(r"C:\managed\node_modules");
        let old_target = r"C:\old\node_modules\@zebbkira\dsh-skills-mcp-manager";
        let mut managed = BTreeMap::new();
        managed.insert(
            LEGACY_SKILLS_MCP_BUNDLE.to_owned(),
            ManagedPluginState {
                version: "0.1.3".to_owned(),
                link_target: old_target.to_owned(),
                bundle_enabled: false,
            },
        );
        let state = PluginInstallState {
            schema_version: 1,
            lock_digest: "old".to_owned(),
            managed,
            managed_skills: BTreeMap::new(),
            sidebar_defaults_seeded: true,
        };
        let profile = json!({
          "dependencies": {
            LEGACY_SKILLS_MCP_BUNDLE: "link:C:/old/node_modules/@zebbkira/dsh-skills-mcp-manager"
          },
          "dsh": {"profile": {"bundles": [BASE_BUNDLE, MARKET_BUNDLE]}}
        });

        let plan = plan_profile(profile, &state, &lock(), store, "new").unwrap();

        assert_eq!(plan.removed_packages, vec![LEGACY_SKILLS_MCP_BUNDLE]);
        assert!(plan.profile["dependencies"]
            .get(LEGACY_SKILLS_MCP_BUNDLE)
            .is_none());
        assert!(plan.profile["dependencies"][SKILLS_MCP_BUNDLE]
            .as_str()
            .unwrap()
            .starts_with("link:"));
        assert!(!bundles(&plan.profile).contains(&SKILLS_MCP_BUNDLE));
        assert!(!plan.next_state.managed[SKILLS_MCP_BUNDLE].bundle_enabled);
    }

    #[test]
    fn legacy_side_panel_is_deactivated_without_removing_its_dependency() {
        let profile = json!({
          "dependencies": {LEGACY_SIDE_PANEL: "1.0.0"},
          "dsh": {"profile": {"bundles": [BASE_BUNDLE, LEGACY_SIDE_PANEL]}}
        });
        let plan = plan_profile(
            profile,
            &PluginInstallState::default(),
            &lock(),
            Path::new(r"C:\managed\node_modules"),
            "digest-d",
        )
        .unwrap();

        assert!(!bundles(&plan.profile).contains(&LEGACY_SIDE_PANEL));
        assert_eq!(plan.profile["dependencies"][LEGACY_SIDE_PANEL], "1.0.0");
    }

    #[test]
    fn repeating_the_same_migration_is_idempotent() {
        let first = plan_profile(
            json!({}),
            &PluginInstallState::default(),
            &lock(),
            Path::new(r"C:\managed\node_modules"),
            "digest-e",
        )
        .unwrap();
        let second = plan_profile(
            first.profile.clone(),
            &first.next_state,
            &lock(),
            Path::new(r"C:\managed\node_modules"),
            "digest-e",
        )
        .unwrap();

        assert_eq!(first.profile, second.profile);
        assert_eq!(first.next_state, second.next_state);
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "dsh-desktop-plugin-{name}-{}-{}",
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

        fn write(&self, relative: &str, content: impl AsRef<[u8]>) {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Default)]
    struct FakeLinker {
        links: Mutex<BTreeMap<PathBuf, PathBuf>>,
    }

    impl DirectoryLinker for FakeLinker {
        fn target(&self, link: &Path) -> Result<Option<PathBuf>, String> {
            Ok(self.links.lock().unwrap().get(link).cloned())
        }

        fn create(&self, link: &Path, target: &Path) -> Result<(), String> {
            self.links
                .lock()
                .unwrap()
                .insert(link.to_owned(), target.to_owned());
            Ok(())
        }

        fn remove(&self, link: &Path) -> Result<(), String> {
            self.links.lock().unwrap().remove(link);
            Ok(())
        }
    }

    /// 在创建首个链接时模拟用户或 pnpm 并发修改 profile。
    struct ConcurrentProfileLinker {
        links: Mutex<BTreeMap<PathBuf, PathBuf>>,
        profile: PathBuf,
    }

    impl DirectoryLinker for ConcurrentProfileLinker {
        fn target(&self, link: &Path) -> Result<Option<PathBuf>, String> {
            Ok(self.links.lock().unwrap().get(link).cloned())
        }

        fn create(&self, link: &Path, target: &Path) -> Result<(), String> {
            if !link.ends_with(MARKET_RUNTIME_ALIAS) && !link.ends_with(MARKET_BUNDLE) {
                fs::write(&self.profile, br#"{"name":"written-by-user"}"#).unwrap();
            }
            self.links
                .lock()
                .unwrap()
                .insert(link.to_owned(), target.to_owned());
            Ok(())
        }

        fn remove(&self, link: &Path) -> Result<(), String> {
            self.links.lock().unwrap().remove(link);
            Ok(())
        }
    }

    fn manager_fixture() -> (TestDirectory, PluginManager, Arc<FakeLinker>) {
        let root = TestDirectory::new("manager");
        let resources = root.path().join("resources/plugins");
        let dsh_home = root.path().join("home/.dsh");
        let web_profile = dsh_home.join("profiles/web");
        let managed = dsh_home.join("profiles/node_modules/.dsh-desktop");
        let bundled_market_root = root.path().join("runtime/host/node_modules/dshmarket");
        root.write(
            "resources/plugins/plugins.lock.json",
            br#"{
              "schemaVersion":1,
              "plugins":[
                {"package":"dsh-better-sidebar","version":"0.12.2","bundleId":"better-sidebar","license":"MIT","source":{"type":"npm","integrity":"sha512-iKOgZ1auSGj2TyIjsS2nDqYiHrGWHUg08CxcIzgnkRjDyCjb/qjpt6W3cMLAj4KxTD2643+E7dg3nikClO0Esg=="},"requiredFiles":["lib/index.js","cordis.patch.yml"]}
              ]
            }"#,
        );
        root.write(
            "resources/plugins/node_modules/dsh-better-sidebar/package.json",
            br#"{"name":"dsh-better-sidebar","version":"0.12.2"}"#,
        );
        root.write(
            "resources/plugins/node_modules/dsh-better-sidebar/lib/index.js",
            b"export {}",
        );
        root.write(
            "resources/plugins/node_modules/dsh-better-sidebar/cordis.patch.yml",
            b"- insert: []",
        );
        root.write(
            "runtime/host/node_modules/dshmarket/package.json",
            br#"{"name":"dshmarket","version":"1.9.0"}"#,
        );
        root.write(
            "runtime/host/node_modules/dshmarket-desktop/package.json",
            br#"{"name":"dshmarket-desktop","version":"1.6.0"}"#,
        );
        let linker = Arc::new(FakeLinker::default());
        let manager = PluginManager::with_linker(
            resources,
            dsh_home,
            web_profile,
            managed,
            bundled_market_root,
            root.path().join("home"),
            linker.clone(),
        );
        (root, manager, linker)
    }

    fn managed_files_fixture() -> (TestDirectory, PluginManager) {
        let root = TestDirectory::new("managed-files");
        let resources = root.path().join("resources/plugins");
        let dsh_home = root.path().join("home/.dsh");
        let skill_content = b"---\nname: genui\ndescription: test\n---\n";
        let skill_digest = sha256_hex(skill_content);
        let lock = format!(
            r#"{{
              "schemaVersion":1,
              "plugins":[
                {{"package":"@omdsh-dev/dsh-genui","version":"0.8.4","bundleId":"genui","license":"MIT","source":{{"type":"npm","integrity":"sha512-iKOgZ1auSGj2TyIjsS2nDqYiHrGWHUg08CxcIzgnkRjDyCjb/qjpt6W3cMLAj4KxTD2643+E7dg3nikClO0Esg=="}},"requiredFiles":["lib/index.js","SKILL.md"]}},
                {{"package":"@vectorize-io/hindsight-coding-agents","version":"0.3.4","bundleId":"hindsight","license":"MIT","source":{{"type":"npm","integrity":"sha512-iKOgZ1auSGj2TyIjsS2nDqYiHrGWHUg08CxcIzgnkRjDyCjb/qjpt6W3cMLAj4KxTD2643+E7dg3nikClO0Esg=="}},"requiredFiles":["dist/dsh.js"]}}
              ],
              "skills":[{{"name":"genui","sourcePackage":"@omdsh-dev/dsh-genui","sourceFile":"SKILL.md","version":"0.8.4","sha256":"{skill_digest}"}}]
            }}"#
        );
        root.write("resources/plugins/plugins.lock.json", lock);
        root.write(
            "resources/plugins/node_modules/@omdsh-dev/dsh-genui/package.json",
            br#"{"name":"@omdsh-dev/dsh-genui","version":"0.8.4"}"#,
        );
        root.write(
            "resources/plugins/node_modules/@omdsh-dev/dsh-genui/lib/index.js",
            b"export {}",
        );
        root.write(
            "resources/plugins/node_modules/@omdsh-dev/dsh-genui/SKILL.md",
            skill_content,
        );
        root.write(
            "resources/plugins/node_modules/@vectorize-io/hindsight-coding-agents/package.json",
            br#"{"name":"@vectorize-io/hindsight-coding-agents","version":"0.3.4"}"#,
        );
        root.write(
            "resources/plugins/node_modules/@vectorize-io/hindsight-coding-agents/dist/dsh.js",
            b"export {}",
        );
        root.write(
            "runtime/host/node_modules/dshmarket/package.json",
            br#"{"name":"dshmarket","version":"1.9.0"}"#,
        );
        root.write(
            "runtime/host/node_modules/dshmarket-desktop/package.json",
            br#"{"name":"dshmarket-desktop","version":"1.6.0"}"#,
        );
        let manager = PluginManager::with_linker(
            resources,
            dsh_home.clone(),
            dsh_home.join("profiles/web"),
            dsh_home.join("profiles/node_modules/.dsh-desktop"),
            root.path().join("runtime/host/node_modules/dshmarket"),
            root.path().join("home"),
            Arc::new(FakeLinker::default()),
        );
        (root, manager)
    }

    #[test]
    fn prepare_then_commit_writes_profile_links_and_marker() {
        let (root, manager, linker) = manager_fixture();
        let transaction = manager.prepare().unwrap();
        assert!(transaction.should_seed_sidebar());
        assert!(root
            .path()
            .join("home/.dsh/profiles/web/package.json")
            .is_file());
        assert!(!root
            .path()
            .join("home/.dsh/desktop-managed/plugins-state.json")
            .exists());
        assert_eq!(linker.links.lock().unwrap().len(), 2);
        assert!(linker.links.lock().unwrap().contains_key(
            &root
                .path()
                .join("home/.dsh/profiles/web/node_modules/dshmarket")
        ));
        assert!(!linker.links.lock().unwrap().contains_key(
            &root
                .path()
                .join("home/.dsh/profiles/node_modules/dshmarket-desktop")
        ));

        transaction.commit().unwrap();
        assert!(root
            .path()
            .join("home/.dsh/desktop-managed/plugins-state.json")
            .is_file());
        assert!(!root
            .path()
            .join("home/.dsh/profiles/web/cordis.patch.yml")
            .exists());
    }

    #[test]
    fn repeated_prepare_preserves_profile_and_marker_bytes_and_mtime() {
        let (root, manager, _linker) = manager_fixture();
        manager.prepare().unwrap().commit().unwrap();
        let profile = root.path().join("home/.dsh/profiles/web/package.json");
        let marker = root
            .path()
            .join("home/.dsh/desktop-managed/plugins-state.json");
        let profile_bytes = fs::read(&profile).unwrap();
        let marker_bytes = fs::read(&marker).unwrap();
        let profile_modified = fs::metadata(&profile).unwrap().modified().unwrap();
        let marker_modified = fs::metadata(&marker).unwrap().modified().unwrap();

        manager.prepare().unwrap().commit().unwrap();

        assert_eq!(fs::read(&profile).unwrap(), profile_bytes);
        assert_eq!(fs::read(&marker).unwrap(), marker_bytes);
        assert_eq!(
            fs::metadata(&profile).unwrap().modified().unwrap(),
            profile_modified
        );
        assert_eq!(
            fs::metadata(&marker).unwrap().modified().unwrap(),
            marker_modified
        );
    }

    #[test]
    fn user_market_dependency_prevents_desktop_runtime_link() {
        let (root, manager, linker) = manager_fixture();
        root.write(
            "home/.dsh/profiles/web/package.json",
            br#"{"dependencies":{"dshmarket":"1.8.0"},"dsh":{"profile":{"bundles":["@deepseek-ai/dsh-base"]}}}"#,
        );

        let transaction = manager.prepare().unwrap();
        let profile: Value = serde_json::from_slice(
            &fs::read(root.path().join("home/.dsh/profiles/web/package.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(profile["dependencies"][MARKET_BUNDLE], "1.8.0");
        assert!(!linker.links.lock().unwrap().contains_key(
            &root
                .path()
                .join("home/.dsh/profiles/web/node_modules/dshmarket")
        ));
        transaction.rollback().unwrap();
    }

    #[test]
    fn prepare_repairs_existing_skin_patch_before_host_startup() {
        let (root, manager, _linker) = manager_fixture();
        root.write(
            "home/.dsh/cordis.patch.yml",
            b"[]\n\n# --- dsh-skin managed (auto-generated; do not edit) ---\n- id: ui-skin-blue-fantasy\n  disabled: true\n",
        );

        let transaction = manager.prepare().unwrap();
        let repaired = fs::read_to_string(root.path().join("home/.dsh/cordis.patch.yml")).unwrap();
        assert!(repaired.starts_with("# --- dsh-skin managed"));
        assert!(serde_yaml::from_str::<serde_yaml::Value>(&repaired)
            .unwrap()
            .is_sequence());
        transaction.rollback().unwrap();
        // 历史坏文件修复独立于插件事务，core fallback 也必须继续使用合法 patch。
        assert!(
            !fs::read_to_string(root.path().join("home/.dsh/cordis.patch.yml"))
                .unwrap()
                .starts_with("[]")
        );
    }

    #[test]
    fn rollback_restores_original_profile_and_removes_new_links() {
        let (root, manager, linker) = manager_fixture();
        let original = br#"{"name":"existing","dependencies":{},"dsh":{"profile":{"bundles":["@deepseek-ai/dsh-base"]}}}"#;
        root.write("home/.dsh/profiles/web/package.json", original);

        let transaction = manager.prepare().unwrap();
        transaction.rollback().unwrap();

        let restored: Value = serde_json::from_slice(
            &fs::read(root.path().join("home/.dsh/profiles/web/package.json")).unwrap(),
        )
        .unwrap();
        assert!(restored["dependencies"][MARKET_BUNDLE]
            .as_str()
            .unwrap()
            .starts_with("link:"));
        assert_eq!(bundles(&restored), vec![BASE_BUNDLE, MARKET_BUNDLE]);
        assert_eq!(linker.links.lock().unwrap().len(), 1);
        assert!(!root
            .path()
            .join("home/.dsh/desktop-managed/plugins-state.json")
            .exists());
    }

    #[test]
    fn rollback_removes_new_managed_skill_and_hindsight_config() {
        let (root, manager) = managed_files_fixture();
        let transaction = manager.prepare().unwrap();
        let skill = root.path().join("home/.dsh/skills/genui/SKILL.md");
        let hindsight = root.path().join("home/.hindsight/coding-agent.json");
        assert!(skill.is_file());
        assert!(hindsight.is_file());
        let config: Value = serde_json::from_slice(&fs::read(&hindsight).unwrap()).unwrap();
        assert_eq!(config["harnesses"]["dsh"]["optInOnly"], true);
        assert_eq!(config["harnesses"]["dsh"]["optInPaths"], json!([]));

        transaction.rollback().unwrap();
        assert!(!skill.exists());
        assert!(!hindsight.exists());
    }

    #[test]
    fn existing_hindsight_config_and_user_modified_skill_are_preserved() {
        let (root, manager) = managed_files_fixture();
        root.write(
            "home/.hindsight/coding-agent.json",
            br#"{"apiUrl":"http://127.0.0.1:8888"}"#,
        );
        root.write("home/.dsh/skills/genui/SKILL.md", b"user-owned");

        let transaction = manager.prepare().unwrap();
        transaction.commit().unwrap();

        assert_eq!(
            fs::read(root.path().join("home/.hindsight/coding-agent.json")).unwrap(),
            br#"{"apiUrl":"http://127.0.0.1:8888"}"#
        );
        assert_eq!(
            fs::read(root.path().join("home/.dsh/skills/genui/SKILL.md")).unwrap(),
            b"user-owned"
        );
        let state: PluginInstallState = serde_json::from_slice(
            &fs::read(
                root.path()
                    .join("home/.dsh/desktop-managed/plugins-state.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(!state.managed_skills.contains_key("genui"));
    }

    #[test]
    fn missing_required_plugin_file_keeps_only_builtin_market_profile() {
        let (root, manager, _linker) = manager_fixture();
        fs::remove_file(
            root.path()
                .join("resources/plugins/node_modules/dsh-better-sidebar/lib/index.js"),
        )
        .unwrap();

        let error = manager.prepare().err().unwrap();
        assert!(error.contains("required file"));
        let profile: Value = serde_json::from_slice(
            &fs::read(root.path().join("home/.dsh/profiles/web/package.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            bundles(&profile),
            vec![BASE_BUNDLE, WEB_APP_BUNDLE, MARKET_BUNDLE]
        );
    }

    #[test]
    fn concurrent_profile_change_is_preserved_and_prepared_links_are_rolled_back() {
        let (root, _, _) = manager_fixture();
        let dsh_home = root.path().join("home/.dsh");
        let web_profile = dsh_home.join("profiles/web");
        let profile_path = web_profile.join("package.json");
        root.write(
            "home/.dsh/profiles/web/package.json",
            br#"{"name":"original"}"#,
        );
        let linker = Arc::new(ConcurrentProfileLinker {
            links: Mutex::new(BTreeMap::new()),
            profile: profile_path.clone(),
        });
        let manager = PluginManager::with_linker(
            root.path().join("resources/plugins"),
            dsh_home.clone(),
            web_profile,
            dsh_home.join("profiles/node_modules/.dsh-desktop"),
            root.path().join("runtime/host/node_modules/dshmarket"),
            root.path().join("home"),
            linker.clone(),
        );

        let error = manager.prepare().err().unwrap();
        assert!(error.contains("concurrently"));
        assert_eq!(
            fs::read(profile_path).unwrap(),
            br#"{"name":"written-by-user"}"#
        );
        assert_eq!(linker.links.lock().unwrap().len(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn plugin_store_copy_rejects_directory_junctions() {
        let root = TestDirectory::new("copy-junction");
        let source = root.path().join("source");
        let outside = root.path().join("outside");
        let link = source.join("linked");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("payload.js"), b"export {};").unwrap();
        super::create_directory_link(Path::new("node.exe"), &link, &outside).unwrap();

        let error = copy_physical_tree(&source, &root.path().join("destination")).unwrap_err();
        assert!(error.contains("must not contain links"));
        fs::remove_dir(&link).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn production_linker_creates_a_real_windows_junction() {
        let root = TestDirectory::new("junction");
        let target = root.path().join("target");
        let link = root.path().join("profile/node_modules/plugin");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        let linker = super::SystemDirectoryLinker {
            node: PathBuf::from("node.exe"),
        };

        linker.create(&link, &target).unwrap();
        assert_eq!(
            linker.target(&link).unwrap().unwrap(),
            fs::canonicalize(&target).unwrap()
        );
        linker.remove(&link).unwrap();
        assert!(target.is_dir());
        assert!(!link.exists());
    }
}
