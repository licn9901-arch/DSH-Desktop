//! 桌面托管插件的锁文件、profile 迁移和失败回滚。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::runtime::RuntimePaths;

const SUPPORTED_SCHEMA_VERSION: u32 = 1;
const BASE_BUNDLE: &str = "@deepseek-ai/dsh-base";
const WEB_APP_BUNDLE: &str = "@deepseek-ai/dsh-web-app";
const LEGACY_SIDE_PANEL: &str = "@dsh-external/dsh-side-panel";

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
    pub sidebar_defaults_seeded: bool,
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
        linker: Arc<dyn DirectoryLinker>,
    ) -> Self {
        Self {
            resources,
            dsh_home,
            web_profile,
            managed_plugins_root,
            linker,
        }
    }

    /// 校验资源并准备 profile；任何中途错误都必须保持原状态。
    pub fn prepare(&self) -> Result<PluginTransaction, String> {
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
        let plan = plan_profile(profile.clone(), &state, &lock, &store_modules, &digest)?;
        if plan.profile != profile && profile_path.is_file() {
            persist_profile_backup(&self.dsh_home, &profile_path)?;
        }
        let expected_profile_bytes = read_optional_bytes(&profile_path)?;
        let installs_skins = plan.next_state.managed.contains_key("@linxin666/dsh-skins");

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
            for (name, content) in [
                (
                    "cordis.patch.yml",
                    b"# DSH web profile user patch layer.\n[]\n".as_slice(),
                ),
                (
                    "pnpm-workspace.yaml",
                    b"packages:\n  - .\n\nnodeLinker: hoisted\nautoInstallPeers: false\n"
                        .as_slice(),
                ),
            ] {
                let path = self.web_profile.join(name);
                if !path.exists() {
                    transaction.snapshots.push(FileSnapshot::capture(&path)?);
                    atomic_write(&path, content)?;
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

            // Skin Center 首次写入前确保 patch 文件存在；回滚时仍恢复用户原状态。
            if installs_skins {
                let patch_path = self.dsh_home.join("cordis.patch.yml");
                if !patch_path.exists() {
                    transaction
                        .snapshots
                        .push(FileSnapshot::capture(&patch_path)?);
                    atomic_write(&patch_path, b"[]\n")?;
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
        atomic_write_json(&self.state_path, &self.next_state)?;
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
        if !link.exists() {
            return Ok(None);
        }
        let metadata = fs::symlink_metadata(link)
            .map_err(|error| format!("failed to inspect {}: {error}", link.display()))?;
        if !is_directory_link(&metadata) {
            return Ok(None);
        }
        fs::canonicalize(link)
            .map(Some)
            .map_err(|error| format!("failed to resolve {}: {error}", link.display()))
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
        copy_physical_tree(
            &resources.join("node_modules"),
            &staging.join("node_modules"),
        )?;
        atomic_write(&staging.join("plugins.lock.json"), lock_bytes)?;
        validate_plugin_tree(&staging.join("node_modules"), lock)?;
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

/// 递归复制普通文件和目录，并拒绝资源包中的链接与 reparse point。
fn copy_physical_tree(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("failed to inspect {}: {error}", source.display()))?;
    if is_directory_link(&metadata) || metadata.file_type().is_symlink() {
        return Err(format!(
            "plugin resources must not contain links: {}",
            source.display()
        ));
    }
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
    if !metadata.is_dir() {
        return Err(format!("unsupported plugin resource: {}", source.display()));
    }
    fs::create_dir_all(destination)
        .map_err(|error| format!("failed to create {}: {error}", destination.display()))?;
    for entry in fs::read_dir(source)
        .map_err(|error| format!("failed to read {}: {error}", source.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read plugin resource: {error}"))?;
        copy_physical_tree(&entry.path(), &destination.join(entry.file_name()))?;
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
        Ok(lock)
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
    deduplicate(&mut current_bundles);

    let mut next_state = PluginInstallState {
        schema_version: SUPPORTED_SCHEMA_VERSION,
        lock_digest: lock_digest.to_owned(),
        managed: BTreeMap::new(),
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
        let bundle_enabled = previous
            .map(|_| {
                current_bundles
                    .iter()
                    .any(|bundle| bundle == &plugin.package)
            })
            .unwrap_or(true);
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

    let managed_bundles = lock
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
    let official_prefix = current_bundles
        .iter()
        .take_while(|bundle| bundle.starts_with("@deepseek-ai/"))
        .count();
    current_bundles.splice(official_prefix..official_prefix, managed_bundles);
    deduplicate(&mut current_bundles);

    *object_field(root, "dependencies")? = dependency_values;
    *profile_bundles(root)? = current_bundles.into_iter().map(Value::String).collect();
    Ok(ProfilePlan {
        profile,
        next_state,
        managed_packages,
    })
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
    }
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
        plan_profile, DirectoryLinker, ManagedPluginState, PluginInstallState, PluginLock,
        PluginManager, BASE_BUNDLE, LEGACY_SIDE_PANEL, WEB_APP_BUNDLE,
    };

    fn lock() -> PluginLock {
        PluginLock::parse(
            br#"{
              "schemaVersion": 1,
              "sharedPackages": ["@deepseek-ai", "react", "react-dom"],
              "plugins": [
                {"package":"dsh-at-file","version":"0.6.0","bundleId":"dsh-at-file","license":"MIT","source":{"type":"npm","integrity":"sha512-iKOgZ1auSGj2TyIjsS2nDqYiHrGWHUg08CxcIzgnkRjDyCjb/qjpt6W3cMLAj4KxTD2643+E7dg3nikClO0Esg=="},"requiredFiles":["lib/index.js"]},
                {"package":"@omdsh-dev/dsh-genui","version":"0.8.4","bundleId":"genui","license":"MIT","source":{"type":"npm","integrity":"sha512-iKOgZ1auSGj2TyIjsS2nDqYiHrGWHUg08CxcIzgnkRjDyCjb/qjpt6W3cMLAj4KxTD2643+E7dg3nikClO0Esg=="},"requiredFiles":["lib/index.js"]},
                {"package":"dsh-better-sidebar","version":"0.12.2","bundleId":"better-sidebar","license":"MIT","source":{"type":"npm","integrity":"sha512-iKOgZ1auSGj2TyIjsS2nDqYiHrGWHUg08CxcIzgnkRjDyCjb/qjpt6W3cMLAj4KxTD2643+E7dg3nikClO0Esg=="},"requiredFiles":["lib/index.js"]},
                {"package":"@linxin666/dsh-skins","version":"0.1.16","bundleId":"ui-skin-center","license":"Apache-2.0","source":{"type":"npm","integrity":"sha512-iKOgZ1auSGj2TyIjsS2nDqYiHrGWHUg08CxcIzgnkRjDyCjb/qjpt6W3cMLAj4KxTD2643+E7dg3nikClO0Esg=="},"requiredFiles":["cordis.patch.yml"]}
              ]
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
                "dsh-at-file",
                "@omdsh-dev/dsh-genui",
                "dsh-better-sidebar",
                "@linxin666/dsh-skins"
            ]
        );
        assert_eq!(plan.managed_packages.len(), 4);
        assert_eq!(plan.next_state.lock_digest, "digest-a");
        assert!(plan.profile["dependencies"]["dsh-at-file"]
            .as_str()
            .unwrap()
            .starts_with("link:"));
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
                "dsh-at-file",
                "@omdsh-dev/dsh-genui",
                "dsh-better-sidebar",
                "@linxin666/dsh-skins",
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
            fs::write(&self.profile, br#"{"name":"written-by-user"}"#).unwrap();
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
        let linker = Arc::new(FakeLinker::default());
        let manager =
            PluginManager::with_linker(resources, dsh_home, web_profile, managed, linker.clone());
        (root, manager, linker)
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
        assert_eq!(linker.links.lock().unwrap().len(), 1);

        transaction.commit().unwrap();
        assert!(root
            .path()
            .join("home/.dsh/desktop-managed/plugins-state.json")
            .is_file());
    }

    #[test]
    fn rollback_restores_original_profile_and_removes_new_links() {
        let (root, manager, linker) = manager_fixture();
        let original = br#"{"name":"existing","dependencies":{},"dsh":{"profile":{"bundles":["@deepseek-ai/dsh-base"]}}}"#;
        root.write("home/.dsh/profiles/web/package.json", original);

        let transaction = manager.prepare().unwrap();
        transaction.rollback().unwrap();

        assert_eq!(
            fs::read(root.path().join("home/.dsh/profiles/web/package.json")).unwrap(),
            original
        );
        assert!(linker.links.lock().unwrap().is_empty());
        assert!(!root
            .path()
            .join("home/.dsh/desktop-managed/plugins-state.json")
            .exists());
    }

    #[test]
    fn missing_required_plugin_file_fails_without_touching_profile() {
        let (root, manager, _linker) = manager_fixture();
        fs::remove_file(
            root.path()
                .join("resources/plugins/node_modules/dsh-better-sidebar/lib/index.js"),
        )
        .unwrap();

        let error = manager.prepare().err().unwrap();
        assert!(error.contains("required file"));
        assert!(!root
            .path()
            .join("home/.dsh/profiles/web/package.json")
            .exists());
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
            linker.clone(),
        );

        let error = manager.prepare().err().unwrap();
        assert!(error.contains("concurrently"));
        assert_eq!(
            fs::read(profile_path).unwrap(),
            br#"{"name":"written-by-user"}"#
        );
        assert!(linker.links.lock().unwrap().is_empty());
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
