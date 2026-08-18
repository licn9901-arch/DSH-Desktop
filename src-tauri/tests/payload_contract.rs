//! Payload 打包、校验与运行时状态的跨模块契约测试。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use dsh_desktop::payload::{
    calculate_payload_digest, create_deterministic_zip, garbage_collect_runtimes, inspect_zip,
    promote_candidate, provision_payload, read_runtime_state, reject_candidate,
    rollback_candidate_promotion, write_runtime_state, ArchiveDescriptor, PayloadEntries,
    PayloadManifest, RuntimeSlot, RuntimeState, ZipLimits,
};

/// 创建退出作用域后自动清理的测试目录。
fn test_directory(name: &str) -> TestDirectory {
    let path = std::env::temp_dir().join(format!(
        "dsh-desktop-payload-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&path).expect("应创建测试目录");
    TestDirectory(path)
}

/// 保存测试目录，并在测试结束时递归删除。
struct TestDirectory(PathBuf);

impl TestDirectory {
    /// 返回测试目录路径。
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// 构造最小合法 manifest，方便验证摘要的固定输入顺序。
fn manifest() -> PayloadManifest {
    PayloadManifest {
        schema_version: 1,
        runtime_abi: 1,
        desktop_version: "0.1.0-preview.8".to_owned(),
        payload_digest: String::new(),
        node_version: "22.22.3".to_owned(),
        pnpm_version: "10.34.5".to_owned(),
        entries: PayloadEntries {
            node: "node/node.exe".to_owned(),
            host: "host/node_modules/@deepseek-ai/dsh/lib/bin.js".to_owned(),
            plugins: "plugins/node_modules".to_owned(),
        },
        node_runtime: ArchiveDescriptor::for_test("node-runtime.zip", "11"),
        host_runtime: ArchiveDescriptor::for_test("host-runtime.zip", "22"),
        builtin_plugins: ArchiveDescriptor::for_test("builtin-plugins.zip", "33"),
    }
}

#[test]
fn payload_digest_uses_schema_abi_and_fixed_archive_order() {
    let original = calculate_payload_digest(&manifest()).expect("应计算 payload 摘要");
    let mut changed = manifest();
    changed.host_runtime.sha256 = "44".repeat(32);
    let changed_digest = calculate_payload_digest(&changed).expect("应计算修改后的摘要");

    assert_eq!(original.len(), 64);
    assert_ne!(original, changed_digest);
}

#[test]
fn deterministic_zip_is_byte_identical_for_equal_content() {
    let root = test_directory("deterministic");
    let input = root.path().join("input");
    fs::create_dir_all(input.join("nested")).expect("应创建输入目录");
    fs::write(input.join("z.txt"), b"z").expect("应写入文件");
    fs::write(input.join("nested/a.txt"), b"a").expect("应写入文件");
    let first = root.path().join("first.zip");
    let second = root.path().join("second.zip");

    let first_descriptor =
        create_deterministic_zip(&input, &first, "payload.zip").expect("第一次打包应成功");
    let second_descriptor =
        create_deterministic_zip(&input, &second, "payload.zip").expect("第二次打包应成功");

    assert_eq!(fs::read(first).unwrap(), fs::read(second).unwrap());
    assert_eq!(first_descriptor, second_descriptor);
    assert_eq!(first_descriptor.file_count, 2);
}

#[test]
fn zip_inspection_rejects_windows_unsafe_paths_and_duplicates() {
    for entry in [
        "../escape.txt",
        "/absolute.txt",
        "C:/drive.txt",
        "safe/file.txt:stream",
        "safe/CON.txt",
        "safe/trailing. ",
    ] {
        let root = test_directory("unsafe");
        let archive = root.path().join("unsafe.zip");
        write_raw_zip(&archive, &[(entry, b"bad")]);
        let error = inspect_zip(&archive, ZipLimits::default()).expect_err("危险路径必须被拒绝");
        assert!(
            error.contains("ZIP"),
            "unexpected error for {entry}: {error}"
        );
    }

    let root = test_directory("duplicate");
    let archive = root.path().join("duplicate.zip");
    write_raw_zip(
        &archive,
        &[("Safe/File.txt", b"a"), ("safe/file.TXT", b"b")],
    );
    let error = inspect_zip(&archive, ZipLimits::default()).expect_err("大小写冲突必须被拒绝");
    assert!(error.contains("conflict") || error.contains("duplicate"));
}

#[test]
fn zip_inspection_enforces_file_count_and_unpacked_size_limits() {
    let root = test_directory("limits");
    let archive = root.path().join("limits.zip");
    write_raw_zip(&archive, &[("one.txt", b"1234"), ("two.txt", b"5678")]);

    assert!(inspect_zip(
        &archive,
        ZipLimits {
            max_files: 1,
            max_unpacked_bytes: 1024,
        }
    )
    .is_err());
    assert!(inspect_zip(
        &archive,
        ZipLimits {
            max_files: 10,
            max_unpacked_bytes: 7,
        }
    )
    .is_err());
}

#[test]
fn runtime_state_keeps_active_previous_and_candidate_together() {
    let active = RuntimeSlot::new("a".repeat(64), 1, "0.1.0-preview.7");
    let candidate = RuntimeSlot::new("b".repeat(64), 1, "0.1.0-preview.8");
    let state = RuntimeState {
        schema_version: 1,
        active: Some(active.clone()),
        previous: None,
        candidate: Some(candidate.clone()),
    };

    let promoted = state.promote_candidate().expect("candidate 应可提升");
    assert_eq!(promoted.active, Some(candidate));
    assert_eq!(promoted.previous, Some(active));
    assert!(promoted.candidate.is_none());
}

#[test]
fn provision_stages_candidate_and_promotion_preserves_previous() {
    let root = test_directory("provision");
    let resources = root.path().join("resources");
    let runtime_root = root.path().join("runtime");
    let expected = write_payload_fixture(&resources);

    let result = provision_payload(&resources, &runtime_root, &[1]).expect("provision 应成功");
    assert_eq!(result.payload_digest, expected.payload_digest);
    assert!(result
        .runtime_directory
        .join(&expected.entries.node)
        .is_file());
    assert!(result
        .runtime_directory
        .join(&expected.entries.host)
        .is_file());
    assert!(result
        .runtime_directory
        .join(&expected.entries.plugins)
        .is_dir());

    let staged = read_runtime_state(&runtime_root).expect("应读取 runtime 状态");
    assert!(staged.active.is_none());
    assert_eq!(
        staged.candidate.as_ref().map(|slot| &slot.payload_digest),
        Some(&expected.payload_digest)
    );

    promote_candidate(&runtime_root, &expected.payload_digest).expect("candidate 应提升");
    let active = read_runtime_state(&runtime_root).expect("应读取提升后的状态");
    assert_eq!(
        active.active.as_ref().map(|slot| &slot.payload_digest),
        Some(&expected.payload_digest)
    );
    assert!(active.candidate.is_none());
}

#[test]
fn failed_plugin_commit_rolls_back_promotion_without_losing_new_candidate() {
    let root = test_directory("promotion-rollback");
    let runtime_root = root.path().join("runtime");
    let old_active = RuntimeSlot::new("a".repeat(64), 1, "old-active");
    let old_previous = RuntimeSlot::new("b".repeat(64), 1, "old-previous");
    let promoted = RuntimeSlot::new("c".repeat(64), 1, "promoted");
    let concurrent = RuntimeSlot::new("d".repeat(64), 1, "concurrent");
    let before = RuntimeState {
        schema_version: 1,
        active: Some(old_active.clone()),
        previous: Some(old_previous.clone()),
        candidate: Some(promoted.clone()),
    };
    write_runtime_state(&runtime_root, &before).expect("应写入提升前状态");
    let snapshot = promote_candidate(&runtime_root, &promoted.payload_digest).expect("应提升");

    let mut changed = read_runtime_state(&runtime_root).expect("应读取提升后状态");
    changed.candidate = Some(concurrent.clone());
    write_runtime_state(&runtime_root, &changed).expect("应模拟并发 provision");
    rollback_candidate_promotion(&runtime_root, &promoted.payload_digest, &snapshot)
        .expect("应恢复提升前 active/previous");

    let restored = read_runtime_state(&runtime_root).expect("应读取恢复状态");
    assert_eq!(restored.active, Some(old_active));
    assert_eq!(restored.previous, Some(old_previous));
    assert_eq!(restored.candidate, Some(concurrent));
}

#[test]
fn corrupted_archive_never_changes_existing_runtime_state() {
    let root = test_directory("corrupt");
    let resources = root.path().join("resources");
    let runtime_root = root.path().join("runtime");
    let manifest = write_payload_fixture(&resources);
    provision_payload(&resources, &runtime_root, &[1]).expect("首次 provision 应成功");
    promote_candidate(&runtime_root, &manifest.payload_digest).expect("首次提升应成功");
    let before = fs::read(runtime_root.join("runtime-state.json")).expect("应读取原状态字节");

    let mut archive = fs::OpenOptions::new()
        .append(true)
        .open(resources.join("host-runtime.zip"))
        .expect("应打开归档");
    archive.write_all(b"corrupted").expect("应损坏归档");
    assert!(provision_payload(&resources, &runtime_root, &[1]).is_err());

    let after = fs::read(runtime_root.join("runtime-state.json")).expect("应读取失败后状态字节");
    assert_eq!(before, after);
}

#[test]
fn manifest_cannot_raise_global_zip_safety_limits() {
    let root = test_directory("global-limits");
    let resources = root.path().join("resources");
    let runtime_root = root.path().join("runtime");
    let mut payload = write_payload_fixture(&resources);
    payload.host_runtime.file_count = ZipLimits::default().max_files + 1;
    payload.payload_digest = calculate_payload_digest(&payload).expect("应重新计算摘要");
    fs::write(
        resources.join("payload-manifest.json"),
        serde_json::to_vec_pretty(&payload).expect("应序列化 manifest"),
    )
    .expect("应写入超限 manifest");

    let error = provision_payload(&resources, &runtime_root, &[1]).expect_err("全局上限必须生效");
    assert!(error.contains("global safety limit"));
}

#[test]
fn reused_runtime_requires_the_original_verified_manifest() {
    let root = test_directory("reused-manifest");
    let resources = root.path().join("resources");
    let runtime_root = root.path().join("runtime");
    let payload = write_payload_fixture(&resources);
    let first = provision_payload(&resources, &runtime_root, &[1]).expect("首次 provision 应成功");
    fs::write(first.runtime_directory.join("payload-manifest.json"), b"{}")
        .expect("应损坏 runtime manifest");

    let error =
        provision_payload(&resources, &runtime_root, &[1]).expect_err("损坏 runtime 不得复用");
    assert!(error.contains("does not match"));
    let state = read_runtime_state(&runtime_root).expect("原 candidate 状态应保留");
    assert_eq!(
        state
            .candidate
            .as_ref()
            .map(|slot| slot.payload_digest.as_str()),
        Some(payload.payload_digest.as_str())
    );
}

#[test]
fn concurrent_provision_serializes_and_reuses_the_same_runtime() {
    let root = test_directory("concurrent");
    let resources = root.path().join("resources");
    let runtime_root = root.path().join("runtime");
    let expected = write_payload_fixture(&resources);
    let first_resources = resources.clone();
    let first_runtime = runtime_root.clone();
    let second_resources = resources.clone();
    let second_runtime = runtime_root.clone();

    let first =
        std::thread::spawn(move || provision_payload(&first_resources, &first_runtime, &[1]));
    let second =
        std::thread::spawn(move || provision_payload(&second_resources, &second_runtime, &[1]));
    let first = first
        .join()
        .expect("线程不应 panic")
        .expect("首次 provision 应成功");
    let second = second
        .join()
        .expect("线程不应 panic")
        .expect("第二次 provision 应成功");

    assert_eq!(first.runtime_directory, second.runtime_directory);
    assert_eq!(first.payload_digest, expected.payload_digest);
    assert!(first.reused || second.reused);
}

#[test]
fn rejection_and_garbage_collection_preserve_only_referenced_runtimes() {
    let root = test_directory("gc");
    let runtime_root = root.path().join("runtime");
    fs::create_dir_all(&runtime_root).expect("应创建 runtime 根目录");
    let active_digest = "a".repeat(64);
    let candidate_digest = "b".repeat(64);
    let stale_digest = "c".repeat(64);
    for digest in [&active_digest, &candidate_digest, &stale_digest] {
        fs::create_dir(runtime_root.join(digest)).expect("应创建 runtime 目录");
    }
    let state = RuntimeState {
        schema_version: 1,
        active: Some(RuntimeSlot::new(&active_digest, 1, "old")),
        previous: None,
        candidate: Some(RuntimeSlot::new(&candidate_digest, 1, "new")),
    };
    dsh_desktop::payload::write_runtime_state(&runtime_root, &state).expect("应写入状态");

    reject_candidate(&runtime_root, &candidate_digest).expect("应拒绝 candidate");
    garbage_collect_runtimes(&runtime_root).expect("垃圾清理应成功");

    assert!(runtime_root.join(&active_digest).is_dir());
    assert!(!runtime_root.join(&candidate_digest).exists());
    assert!(!runtime_root.join(&stale_digest).exists());
}

/// 创建三个最小 ZIP 和与之匹配的 payload manifest。
fn write_payload_fixture(resources: &Path) -> PayloadManifest {
    fs::create_dir_all(resources).expect("应创建资源目录");
    let source = resources.join("source");
    let node = source.join("node");
    let host = source.join("host");
    let plugins = source.join("plugins");
    fs::create_dir_all(node.join("node")).expect("应创建 Node 目录");
    fs::create_dir_all(host.join("host/node_modules/@deepseek-ai/dsh/lib"))
        .expect("应创建 Host 目录");
    fs::create_dir_all(plugins.join("plugins/node_modules/example")).expect("应创建插件目录");
    fs::write(node.join("node/node.exe"), b"node").expect("应写入 Node fixture");
    fs::write(node.join("node/LICENSE"), b"license").expect("应写入许可证 fixture");
    fs::write(
        host.join("host/node_modules/@deepseek-ai/dsh/lib/bin.js"),
        b"console.log('host')",
    )
    .expect("应写入 Host fixture");
    fs::write(
        plugins.join("plugins/node_modules/example/package.json"),
        b"{}",
    )
    .expect("应写入插件 fixture");

    let node_runtime = create_deterministic_zip(
        &node,
        &resources.join("node-runtime.zip"),
        "node-runtime.zip",
    )
    .expect("Node 打包应成功");
    let host_runtime = create_deterministic_zip(
        &host,
        &resources.join("host-runtime.zip"),
        "host-runtime.zip",
    )
    .expect("Host 打包应成功");
    let builtin_plugins = create_deterministic_zip(
        &plugins,
        &resources.join("builtin-plugins.zip"),
        "builtin-plugins.zip",
    )
    .expect("插件打包应成功");
    let mut manifest = PayloadManifest {
        schema_version: 1,
        runtime_abi: 1,
        desktop_version: "0.1.0-preview.8".to_owned(),
        payload_digest: String::new(),
        node_version: "22.22.3".to_owned(),
        pnpm_version: "10.34.5".to_owned(),
        entries: PayloadEntries {
            node: "node/node.exe".to_owned(),
            host: "host/node_modules/@deepseek-ai/dsh/lib/bin.js".to_owned(),
            plugins: "plugins/node_modules".to_owned(),
        },
        node_runtime,
        host_runtime,
        builtin_plugins,
    };
    manifest.payload_digest = calculate_payload_digest(&manifest).expect("应计算 fixture 摘要");
    fs::write(
        resources.join("payload-manifest.json"),
        serde_json::to_vec_pretty(&manifest).expect("应序列化 manifest"),
    )
    .expect("应写入 manifest");
    manifest
}

/// 写入不经过产品路径检查的 ZIP，用于构造恶意归档 fixture。
fn write_raw_zip(path: &Path, entries: &[(&str, &[u8])]) {
    let file = fs::File::create(path).expect("应创建 ZIP fixture");
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    for (name, content) in entries {
        writer.start_file(*name, options).expect("应创建 ZIP 条目");
        writer.write_all(content).expect("应写入 ZIP 条目");
    }
    writer.finish().expect("应完成 ZIP fixture");
}
