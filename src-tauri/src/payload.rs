//! 可复现 payload、ZIP 安全校验和运行时状态契约。

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

/// 当前 payload manifest 的结构版本。
pub const PAYLOAD_SCHEMA_VERSION: u32 = 1;

/// 单个归档在 manifest 中记录的完整性与规模信息。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveDescriptor {
    pub file_name: String,
    pub sha256: String,
    pub compressed_size: u64,
    pub unpacked_size: u64,
    pub file_count: u64,
}

impl ArchiveDescriptor {
    /// 构造测试使用的合法摘要描述，避免 fixture 重复填写规模字段。
    pub fn for_test(file_name: &str, digest_token: &str) -> Self {
        Self {
            file_name: file_name.to_owned(),
            sha256: digest_token.repeat(32),
            compressed_size: 0,
            unpacked_size: 0,
            file_count: 0,
        }
    }
}

/// 运行时三个稳定入口，运行时解析不得依赖目录扫描或搜索路径。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PayloadEntries {
    pub node: String,
    pub host: String,
    pub plugins: String,
}

/// 安装器与运行时共同消费的 payload manifest。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PayloadManifest {
    pub schema_version: u32,
    pub runtime_abi: u32,
    pub desktop_version: String,
    pub payload_digest: String,
    pub node_version: String,
    pub pnpm_version: String,
    pub entries: PayloadEntries,
    pub node_runtime: ArchiveDescriptor,
    pub host_runtime: ArchiveDescriptor,
    pub builtin_plugins: ArchiveDescriptor,
}

impl PayloadManifest {
    /// 校验 manifest 自身结构、固定归档名和 payload 摘要。
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != PAYLOAD_SCHEMA_VERSION {
            return Err(format!(
                "unsupported payload manifest schema: {}",
                self.schema_version
            ));
        }
        if self.runtime_abi == 0 {
            return Err("payload runtime ABI must be positive".to_owned());
        }
        for (descriptor, expected) in [
            (&self.node_runtime, "node-runtime.zip"),
            (&self.host_runtime, "host-runtime.zip"),
            (&self.builtin_plugins, "builtin-plugins.zip"),
        ] {
            if descriptor.file_name != expected {
                return Err(format!(
                    "payload archive name must be {expected}, got {}",
                    descriptor.file_name
                ));
            }
            validate_sha256(&descriptor.sha256)?;
        }
        let calculated = calculate_payload_digest(self)?;
        if self.payload_digest != calculated {
            return Err(format!(
                "payload digest mismatch: expected {}, got {}",
                self.payload_digest, calculated
            ));
        }
        Ok(())
    }
}

/// ZIP 展开前的资源上限，防止文件洪泛和高压缩比耗尽磁盘。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZipLimits {
    pub max_files: u64,
    pub max_unpacked_bytes: u64,
}

impl Default for ZipLimits {
    /// 返回桌面 payload 的保守上限，明显高于当前目标但限制恶意输入。
    fn default() -> Self {
        Self {
            max_files: 50_000,
            max_unpacked_bytes: 2 * 1024 * 1024 * 1024,
        }
    }
}

/// ZIP 校验后的实际规模，供 manifest 对照和日志记录。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZipInspection {
    pub file_count: u64,
    pub unpacked_size: u64,
}

/// 状态文件中的一个不可变运行时版本引用。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSlot {
    pub payload_digest: String,
    pub runtime_abi: u32,
    pub desktop_version: String,
}

impl RuntimeSlot {
    /// 创建一个仅以摘要寻址的运行时引用，不接受任意绝对路径。
    pub fn new(
        payload_digest: impl Into<String>,
        runtime_abi: u32,
        desktop_version: impl Into<String>,
    ) -> Self {
        Self {
            payload_digest: payload_digest.into(),
            runtime_abi,
            desktop_version: desktop_version.into(),
        }
    }

    /// 校验状态引用可安全映射到 runtime 根目录下的摘要目录。
    pub fn validate(&self) -> Result<(), String> {
        validate_sha256(&self.payload_digest)?;
        if self.runtime_abi == 0 {
            return Err("runtime slot ABI must be positive".to_owned());
        }
        Ok(())
    }
}

/// 原子状态文件的完整内容，三个指针始终一起提交。
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeState {
    pub schema_version: u32,
    pub active: Option<RuntimeSlot>,
    pub previous: Option<RuntimeSlot>,
    pub candidate: Option<RuntimeSlot>,
}

/// 一次 provision 的结果，安装器可据此记录是否复用了已有摘要目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionResult {
    pub payload_digest: String,
    pub runtime_directory: PathBuf,
    pub reused: bool,
}

/// 持有 `.provision.lock` 的独占文件锁，离开作用域时自动释放。
struct ProvisionLock {
    file: File,
}

impl ProvisionLock {
    /// 在 runtime 根目录创建并独占 provision 锁，串行化安装器与应用切换。
    fn acquire(runtime_root: &Path) -> Result<Self, String> {
        fs::create_dir_all(runtime_root).map_err(|error| {
            format!(
                "could not create runtime root {}: {error}",
                runtime_root.display()
            )
        })?;
        let path = runtime_root.join(".provision.lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|error| {
                format!("could not open provision lock {}: {error}", path.display())
            })?;
        file.lock_exclusive().map_err(|error| {
            format!(
                "could not acquire provision lock {}: {error}",
                path.display()
            )
        })?;
        Ok(Self { file })
    }
}

impl Drop for ProvisionLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.file);
    }
}

impl RuntimeState {
    /// 将已通过真实 Host 检查的 candidate 提升为 active，并保留上一代。
    pub fn promote_candidate(&self) -> Result<Self, String> {
        let candidate = self
            .candidate
            .clone()
            .ok_or_else(|| "runtime state has no candidate to promote".to_owned())?;
        candidate.validate()?;
        Ok(Self {
            schema_version: self.schema_version.max(1),
            active: Some(candidate),
            previous: self.active.clone(),
            candidate: None,
        })
    }
}

/// 校验并展开安装器 payload，只登记 candidate，不立即切换 active。
pub fn provision_payload(
    resources: &Path,
    runtime_root: &Path,
    supported_abis: &[u32],
) -> Result<ProvisionResult, String> {
    let manifest = verify_payload(resources, supported_abis)?;
    let manifest_path = resources.join("payload-manifest.json");
    let manifest_bytes = fs::read(&manifest_path)
        .map_err(|error| format!("could not reread {}: {error}", manifest_path.display()))?;

    let _lock = ProvisionLock::acquire(runtime_root)?;
    let runtime_directory = runtime_root.join(&manifest.payload_digest);
    let reused = runtime_directory.is_dir();
    if reused {
        let installed_manifest = fs::read(runtime_directory.join("payload-manifest.json"))
            .map_err(|error| format!("could not read existing runtime manifest: {error}"))?;
        if installed_manifest != manifest_bytes {
            return Err("existing runtime manifest does not match the verified payload".to_owned());
        }
        verify_runtime_entries(&runtime_directory, &manifest.entries)?;
    } else {
        let staging = runtime_root.join(format!(
            "{}.staging.{}",
            manifest.payload_digest,
            std::process::id()
        ));
        if staging.exists() {
            fs::remove_dir_all(&staging).map_err(|error| {
                format!(
                    "could not remove stale staging {}: {error}",
                    staging.display()
                )
            })?;
        }
        fs::create_dir(&staging).map_err(|error| {
            format!(
                "could not create runtime staging {}: {error}",
                staging.display()
            )
        })?;
        let staged = (|| {
            for descriptor in [
                &manifest.node_runtime,
                &manifest.host_runtime,
                &manifest.builtin_plugins,
            ] {
                extract_zip(&resources.join(&descriptor.file_name), &staging, descriptor)?;
            }
            verify_runtime_entries(&staging, &manifest.entries)?;
            fs::write(staging.join("payload-manifest.json"), &manifest_bytes)
                .map_err(|error| format!("could not write staged payload manifest: {error}"))?;
            fs::rename(&staging, &runtime_directory).map_err(|error| {
                format!(
                    "could not atomically publish runtime {}: {error}",
                    runtime_directory.display()
                )
            })?;
            Ok::<(), String>(())
        })();
        if let Err(error) = staged {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    }

    let mut state = read_runtime_state(runtime_root)?;
    if state
        .active
        .as_ref()
        .is_some_and(|active| active.payload_digest == manifest.payload_digest)
    {
        state.candidate = None;
    } else {
        state.candidate = Some(RuntimeSlot::new(
            &manifest.payload_digest,
            manifest.runtime_abi,
            &manifest.desktop_version,
        ));
    }
    write_runtime_state(runtime_root, &state)?;

    Ok(ProvisionResult {
        payload_digest: manifest.payload_digest,
        runtime_directory,
        reused,
    })
}

/// 使用与运行时相同的逻辑验证 manifest、ABI 和三个归档，供构建门禁调用。
pub fn verify_payload(resources: &Path, supported_abis: &[u32]) -> Result<PayloadManifest, String> {
    let manifest_path = resources.join("payload-manifest.json");
    let manifest_bytes = fs::read(&manifest_path).map_err(|error| {
        format!(
            "could not read payload manifest {}: {error}",
            manifest_path.display()
        )
    })?;
    if manifest_bytes.len() > 1024 * 1024 {
        return Err("payload manifest exceeds the 1 MiB limit".to_owned());
    }
    let manifest: PayloadManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("invalid payload manifest JSON: {error}"))?;
    manifest.validate()?;
    if !supported_abis.contains(&manifest.runtime_abi) {
        return Err(format!(
            "payload runtime ABI {} is not supported by this desktop build",
            manifest.runtime_abi
        ));
    }
    for descriptor in [
        &manifest.node_runtime,
        &manifest.host_runtime,
        &manifest.builtin_plugins,
    ] {
        verify_archive(resources, descriptor)?;
    }
    Ok(manifest)
}

/// 从单一状态文件读取 active、previous 与 candidate；首次安装返回空状态。
pub fn read_runtime_state(runtime_root: &Path) -> Result<RuntimeState, String> {
    let path = runtime_root.join("runtime-state.json");
    if !path.exists() {
        return Ok(RuntimeState {
            schema_version: 1,
            ..RuntimeState::default()
        });
    }
    let bytes = fs::read(&path)
        .map_err(|error| format!("could not read runtime state {}: {error}", path.display()))?;
    if bytes.len() > 1024 * 1024 {
        return Err("runtime state exceeds the 1 MiB limit".to_owned());
    }
    let state: RuntimeState = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid runtime state JSON: {error}"))?;
    validate_runtime_state(&state)?;
    Ok(state)
}

/// 通过同目录临时文件和原子替换一次性提交完整 runtime 状态。
pub fn write_runtime_state(runtime_root: &Path, state: &RuntimeState) -> Result<(), String> {
    validate_runtime_state(state)?;
    fs::create_dir_all(runtime_root).map_err(|error| {
        format!(
            "could not create runtime root {}: {error}",
            runtime_root.display()
        )
    })?;
    let path = runtime_root.join("runtime-state.json");
    let temporary = runtime_root.join(format!("runtime-state.json.tmp.{}", std::process::id()));
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("could not serialize runtime state: {error}"))?;
    let mut output = File::create(&temporary).map_err(|error| {
        format!(
            "could not create temporary runtime state {}: {error}",
            temporary.display()
        )
    })?;
    use std::io::Write as _;
    output
        .write_all(&bytes)
        .and_then(|()| output.write_all(b"\n"))
        .and_then(|()| output.sync_all())
        .map_err(|error| format!("could not persist temporary runtime state: {error}"))?;
    drop(output);
    if let Err(error) = atomic_replace(&temporary, &path) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

/// 在锁内提升指定 candidate，摘要不一致时拒绝写入，防止陈旧启动覆盖新安装。
pub fn promote_candidate(
    runtime_root: &Path,
    expected_digest: &str,
) -> Result<RuntimeState, String> {
    validate_sha256(expected_digest)?;
    let _lock = ProvisionLock::acquire(runtime_root)?;
    let state = read_runtime_state(runtime_root)?;
    let actual = state
        .candidate
        .as_ref()
        .ok_or_else(|| "runtime state has no candidate to promote".to_owned())?;
    if actual.payload_digest != expected_digest {
        return Err(format!(
            "candidate changed while starting: expected {expected_digest}, got {}",
            actual.payload_digest
        ));
    }
    write_runtime_state(runtime_root, &state.promote_candidate()?)?;
    Ok(state)
}

/// 插件 marker 提交失败时恢复提升前状态，并拒绝刚刚测试失败的 candidate。
///
/// 如果并发 provision 已登记另一 candidate，则保留新 candidate，避免用陈旧快照覆盖安装器结果。
pub fn rollback_candidate_promotion(
    runtime_root: &Path,
    expected_digest: &str,
    previous_state: &RuntimeState,
) -> Result<(), String> {
    validate_sha256(expected_digest)?;
    validate_runtime_state(previous_state)?;
    let _lock = ProvisionLock::acquire(runtime_root)?;
    let current = read_runtime_state(runtime_root)?;
    let active = current
        .active
        .as_ref()
        .ok_or_else(|| "runtime state has no active runtime to roll back".to_owned())?;
    if active.payload_digest != expected_digest {
        return Err(format!(
            "active runtime changed while rolling back: expected {expected_digest}, got {}",
            active.payload_digest
        ));
    }

    let concurrent_candidate = current
        .candidate
        .filter(|slot| slot.payload_digest != expected_digest);
    let restored = RuntimeState {
        schema_version: previous_state.schema_version.max(1),
        active: previous_state.active.clone(),
        previous: previous_state.previous.clone(),
        candidate: concurrent_candidate,
    };
    write_runtime_state(runtime_root, &restored)
}

/// 清除启动失败的指定 candidate；active 与 previous 始终保持不变。
pub fn reject_candidate(runtime_root: &Path, expected_digest: &str) -> Result<(), String> {
    validate_sha256(expected_digest)?;
    let _lock = ProvisionLock::acquire(runtime_root)?;
    let mut state = read_runtime_state(runtime_root)?;
    let actual = state
        .candidate
        .as_ref()
        .ok_or_else(|| "runtime state has no candidate to reject".to_owned())?;
    if actual.payload_digest != expected_digest {
        return Err(format!(
            "candidate changed while rejecting: expected {expected_digest}, got {}",
            actual.payload_digest
        ));
    }
    state.candidate = None;
    write_runtime_state(runtime_root, &state)
}

/// 删除未被 active、previous、candidate 引用的摘要目录和中断 staging。
pub fn garbage_collect_runtimes(runtime_root: &Path) -> Result<Vec<PathBuf>, String> {
    let _lock = ProvisionLock::acquire(runtime_root)?;
    let state = read_runtime_state(runtime_root)?;
    let referenced = [&state.active, &state.previous, &state.candidate]
        .into_iter()
        .flatten()
        .map(|slot| slot.payload_digest.as_str())
        .collect::<BTreeSet<_>>();
    let mut removed = Vec::new();
    for entry in fs::read_dir(runtime_root)
        .map_err(|error| format!("could not list runtime root: {error}"))?
    {
        let entry = entry.map_err(|error| format!("could not read runtime entry: {error}"))?;
        let path = entry.path();
        if !entry
            .file_type()
            .map_err(|error| format!("could not inspect {}: {error}", path.display()))?
            .is_dir()
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_digest = name.len() == 64 && name.bytes().all(|byte| byte.is_ascii_hexdigit());
        let is_staging = name.split_once(".staging.").is_some_and(|(digest, pid)| {
            digest.len() == 64
                && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
                && !pid.is_empty()
                && pid.bytes().all(|byte| byte.is_ascii_digit())
        });
        if (is_digest && !referenced.contains(name.as_str())) || is_staging {
            fs::remove_dir_all(&path)
                .map_err(|error| format!("could not remove runtime {}: {error}", path.display()))?;
            removed.push(path);
        }
    }
    Ok(removed)
}

/// 校验状态结构和所有摘要引用，阻止路径值进入持久化状态。
fn validate_runtime_state(state: &RuntimeState) -> Result<(), String> {
    if state.schema_version != 1 {
        return Err(format!(
            "unsupported runtime state schema: {}",
            state.schema_version
        ));
    }
    for slot in [&state.active, &state.previous, &state.candidate]
        .into_iter()
        .flatten()
    {
        slot.validate()?;
    }
    Ok(())
}

/// 对照 manifest 验证归档哈希、压缩大小和展开规模。
fn verify_archive(resources: &Path, descriptor: &ArchiveDescriptor) -> Result<(), String> {
    let global_limits = ZipLimits::default();
    if descriptor.file_count > global_limits.max_files
        || descriptor.unpacked_size > global_limits.max_unpacked_bytes
    {
        return Err(format!(
            "payload archive exceeds the global safety limit: {}",
            descriptor.file_name
        ));
    }
    let path = resources.join(&descriptor.file_name);
    let metadata = fs::metadata(&path)
        .map_err(|error| format!("payload archive is missing {}: {error}", path.display()))?;
    if metadata.len() != descriptor.compressed_size {
        return Err(format!(
            "payload archive compressed size mismatch for {}",
            descriptor.file_name
        ));
    }
    let digest = sha256_file(&path)?;
    if digest != descriptor.sha256 {
        return Err(format!(
            "payload archive SHA-256 mismatch for {}",
            descriptor.file_name
        ));
    }
    let inspection = inspect_zip(
        &path,
        ZipLimits {
            max_files: descriptor.file_count,
            max_unpacked_bytes: descriptor.unpacked_size,
        },
    )?;
    if inspection.file_count != descriptor.file_count
        || inspection.unpacked_size != descriptor.unpacked_size
    {
        return Err(format!(
            "payload archive contents mismatch for {}",
            descriptor.file_name
        ));
    }
    Ok(())
}

/// 将已校验归档展开到新 staging，实际写入字节数必须匹配声明值。
fn extract_zip(
    archive_path: &Path,
    staging: &Path,
    descriptor: &ArchiveDescriptor,
) -> Result<(), String> {
    let file = File::open(archive_path)
        .map_err(|error| format!("could not open ZIP {}: {error}", archive_path.display()))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| format!("could not parse ZIP {}: {error}", archive_path.display()))?;
    let mut written_files = 0_u64;
    let mut written_bytes = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("could not read ZIP entry {index}: {error}"))?;
        let relative = validate_zip_target(entry.name())?;
        let target = staging.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(|error| {
                format!(
                    "could not create runtime directory {}: {error}",
                    target.display()
                )
            })?;
            continue;
        }
        if entry.encrypted() || entry.is_symlink() || !entry.is_file() {
            return Err(format!(
                "ZIP entry cannot be extracted safely: {}",
                entry.name()
            ));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "could not create runtime directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&target)
            .map_err(|error| {
                format!(
                    "could not create runtime file {} (duplicate target is forbidden): {error}",
                    target.display()
                )
            })?;
        let expected = entry.size();
        let copied = std::io::copy(&mut entry.by_ref().take(expected + 1), &mut output)
            .map_err(|error| format!("could not extract {}: {error}", target.display()))?;
        if copied != expected {
            return Err(format!(
                "ZIP entry size changed while extracting {}: expected {expected}, got {copied}",
                target.display()
            ));
        }
        output
            .sync_all()
            .map_err(|error| format!("could not persist {}: {error}", target.display()))?;
        written_files += 1;
        written_bytes += copied;
    }
    if written_files != descriptor.file_count || written_bytes != descriptor.unpacked_size {
        return Err(format!(
            "extracted archive size mismatch for {}",
            descriptor.file_name
        ));
    }
    Ok(())
}

/// 验证 Node、Host 和插件稳定入口都位于摘要目录内并具有正确类型。
fn verify_runtime_entries(root: &Path, entries: &PayloadEntries) -> Result<(), String> {
    let node = root.join(validate_zip_target(&entries.node)?);
    let host = root.join(validate_zip_target(&entries.host)?);
    let plugins = root.join(validate_zip_target(&entries.plugins)?);
    if !node.is_file() {
        return Err(format!("payload Node entry is missing: {}", node.display()));
    }
    if !host.is_file() {
        return Err(format!("payload Host entry is missing: {}", host.display()));
    }
    if !plugins.is_dir() {
        return Err(format!(
            "payload plugin root is missing: {}",
            plugins.display()
        ));
    }
    Ok(())
}

/// 同目录原子替换状态文件；Windows 使用 MoveFileExW 的替换和落盘标志。
#[cfg(windows)]
fn atomic_replace(source: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source_wide = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let target_wide = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            target_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        return Err(format!(
            "could not atomically replace runtime state {}: {}",
            target.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

/// 非 Windows 测试环境使用同卷 rename 提交状态文件。
#[cfg(not(windows))]
fn atomic_replace(source: &Path, target: &Path) -> Result<(), String> {
    fs::rename(source, target).map_err(|error| {
        format!(
            "could not atomically replace runtime state {}: {error}",
            target.display()
        )
    })
}

/// 按固定字段顺序计算 payload 摘要，避免 JSON 格式差异影响寻址。
pub fn calculate_payload_digest(manifest: &PayloadManifest) -> Result<String, String> {
    for digest in [
        &manifest.node_runtime.sha256,
        &manifest.host_runtime.sha256,
        &manifest.builtin_plugins.sha256,
    ] {
        validate_sha256(digest)?;
    }
    let canonical = format!(
        "schemaVersion={}\nruntimeAbi={}\nnode-runtime.zip={}\nhost-runtime.zip={}\nbuiltin-plugins.zip={}\n",
        manifest.schema_version,
        manifest.runtime_abi,
        manifest.node_runtime.sha256,
        manifest.host_runtime.sha256,
        manifest.builtin_plugins.sha256
    );
    Ok(hex_sha256(canonical.as_bytes()))
}

/// 将目录按路径排序、固定时间戳和 Deflate level 6 写成可复现 ZIP。
pub fn create_deterministic_zip(
    source: &Path,
    output: &Path,
    file_name: &str,
) -> Result<ArchiveDescriptor, String> {
    if !source.is_dir() {
        return Err(format!(
            "payload source directory is missing: {}",
            source.display()
        ));
    }
    let mut files = Vec::new();
    collect_files(source, source, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "could not create ZIP output directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let output_file = File::create(output)
        .map_err(|error| format!("could not create ZIP {}: {error}", output.display()))?;
    let mut writer = ZipWriter::new(output_file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(6))
        .last_modified_time(DateTime::default())
        .unix_permissions(0o644);
    let mut unpacked_size = 0_u64;
    for (relative, absolute, size) in &files {
        writer
            .start_file(relative, options)
            .map_err(|error| format!("could not add {relative} to ZIP: {error}"))?;
        let mut input = BufReader::new(File::open(absolute).map_err(|error| {
            format!(
                "could not read payload file {}: {error}",
                absolute.display()
            )
        })?);
        std::io::copy(&mut input, &mut writer)
            .map_err(|error| format!("could not compress {relative}: {error}"))?;
        unpacked_size = unpacked_size
            .checked_add(*size)
            .ok_or_else(|| "payload unpacked size overflow".to_owned())?;
    }
    writer
        .finish()
        .map_err(|error| format!("could not finish ZIP {}: {error}", output.display()))?;

    Ok(ArchiveDescriptor {
        file_name: file_name.to_owned(),
        sha256: sha256_file(output)?,
        compressed_size: fs::metadata(output)
            .map_err(|error| format!("could not inspect ZIP {}: {error}", output.display()))?
            .len(),
        unpacked_size,
        file_count: files.len() as u64,
    })
}

/// 在展开前检查 ZIP 的路径、条目类型、冲突和资源规模。
pub fn inspect_zip(path: &Path, limits: ZipLimits) -> Result<ZipInspection, String> {
    let file = File::open(path)
        .map_err(|error| format!("could not open ZIP {}: {error}", path.display()))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| format!("could not parse ZIP {}: {error}", path.display()))?;
    let mut targets = BTreeSet::new();
    let mut file_count = 0_u64;
    let mut unpacked_size = 0_u64;

    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format!("could not read ZIP entry {index}: {error}"))?;
        if entry.encrypted() {
            return Err(format!("ZIP entry is encrypted: {}", entry.name()));
        }
        if entry.is_symlink() {
            return Err(format!("ZIP symlink entry is forbidden: {}", entry.name()));
        }
        let normalized = validate_zip_target(entry.name())?;
        let collision_key = normalized.to_lowercase();
        if !targets.insert(collision_key) {
            return Err(format!(
                "ZIP entry has a duplicate or case conflict: {normalized}"
            ));
        }
        if entry.is_dir() {
            continue;
        }
        if !entry.is_file() {
            return Err(format!("ZIP entry type is unsupported: {normalized}"));
        }
        file_count = file_count
            .checked_add(1)
            .ok_or_else(|| "ZIP file count overflow".to_owned())?;
        unpacked_size = unpacked_size
            .checked_add(entry.size())
            .ok_or_else(|| "ZIP unpacked size overflow".to_owned())?;
        if file_count > limits.max_files {
            return Err(format!("ZIP contains too many files: {file_count}"));
        }
        if unpacked_size > limits.max_unpacked_bytes {
            return Err(format!("ZIP unpacked size exceeds limit: {unpacked_size}"));
        }
    }

    Ok(ZipInspection {
        file_count,
        unpacked_size,
    })
}

/// 递归收集普通文件，并拒绝构建输入中的符号链接或 reparse point。
fn collect_files(
    root: &Path,
    directory: &Path,
    output: &mut Vec<(String, PathBuf, u64)>,
) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("could not list {}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("could not list {}: {error}", directory.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
        if metadata.file_type().is_symlink() || metadata_is_reparse_point(&metadata) {
            return Err(format!(
                "payload source symlink or reparse point is forbidden: {}",
                path.display()
            ));
        }
        if metadata.is_dir() {
            collect_files(root, &path, output)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|error| format!("could not relativize {}: {error}", path.display()))?
                .to_string_lossy()
                .replace('\\', "/");
            validate_zip_target(&relative)?;
            output.push((relative, path, metadata.len()));
        } else {
            return Err(format!(
                "payload source type is unsupported: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

/// Windows reparse point 可能不是 Rust 识别的普通 symlink，同样不得进入 payload。
#[cfg(windows)]
fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

/// 非 Windows 平台没有需要额外拒绝的 Windows reparse 属性。
#[cfg(not(windows))]
fn metadata_is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

/// 校验 ZIP 目标符合 Windows 文件系统约束，并返回统一分隔符路径。
fn validate_zip_target(name: &str) -> Result<String, String> {
    if name.is_empty() || name.contains('\0') {
        return Err("ZIP entry has an empty name or NUL byte".to_owned());
    }
    let normalized = name.replace('\\', "/");
    let upper = normalized.to_ascii_uppercase();
    if normalized.starts_with('/')
        || normalized.starts_with("//")
        || upper.starts_with("//?/")
        || upper.starts_with("//./")
    {
        return Err(format!("ZIP entry uses an absolute or device path: {name}"));
    }
    let components = normalized.trim_end_matches('/').split('/');
    let mut found = false;
    for (index, component) in components.enumerate() {
        found = true;
        if component.is_empty() || component == "." || component == ".." {
            return Err(format!(
                "ZIP entry contains an unsafe path component: {name}"
            ));
        }
        if component.ends_with('.') || component.ends_with(' ') {
            return Err(format!("ZIP entry has a trailing dot or space: {name}"));
        }
        if component.contains(':') {
            return Err(format!("ZIP entry contains a drive prefix or ADS: {name}"));
        }
        if index == 0 && component.len() == 2 && component.as_bytes()[1] == b':' {
            return Err(format!("ZIP entry uses a drive path: {name}"));
        }
        let stem = component
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || stem
                .strip_prefix("COM")
                .or_else(|| stem.strip_prefix("LPT"))
                .is_some_and(|suffix| {
                    suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9')
                });
        if reserved {
            return Err(format!(
                "ZIP entry uses a reserved Windows device name: {name}"
            ));
        }
    }
    if !found {
        return Err("ZIP entry has no path components".to_owned());
    }
    Ok(normalized)
}

/// 计算文件 SHA-256，避免把大型归档完整读入内存。
pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut input = BufReader::new(
        File::open(path)
            .map_err(|error| format!("could not open {} for hashing: {error}", path.display()))?,
    );
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|error| format!("could not hash {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format_digest(digest.finalize().as_slice()))
}

/// 校验十六进制 SHA-256 的固定长度和字符集。
fn validate_sha256(value: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("invalid SHA-256 value: {value}"));
    }
    Ok(())
}

/// 对内存字节计算 SHA-256。
fn hex_sha256(value: &[u8]) -> String {
    format_digest(Sha256::digest(value).as_slice())
}

/// 将摘要字节稳定格式化为小写十六进制。
fn format_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}
