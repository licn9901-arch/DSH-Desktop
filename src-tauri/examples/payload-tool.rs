//! 构建期 payload 打包与校验工具。

use std::collections::BTreeMap;
use std::path::PathBuf;

use dsh_desktop::payload::{
    calculate_payload_digest, create_deterministic_zip, verify_payload, PayloadEntries,
    PayloadManifest, PAYLOAD_SCHEMA_VERSION,
};

/// 解析命令并返回适合作为进程退出码的结果。
fn main() {
    if let Err(error) = run(std::env::args().skip(1).collect()) {
        eprintln!("payload-tool failed: {error}");
        std::process::exit(1);
    }
}

/// 执行 package 或 verify 子命令。
fn run(arguments: Vec<String>) -> Result<(), String> {
    let (command, tail) = arguments
        .split_first()
        .ok_or_else(|| "expected package or verify subcommand".to_owned())?;
    let options = parse_options(tail)?;
    match command.as_str() {
        "package" => package(&options),
        "verify" => {
            let resources = required_path(&options, "resources")?;
            let manifest = verify_payload(&resources, &[1])?;
            println!(
                "verified payload {} ({} files, {} unpacked bytes)",
                manifest.payload_digest,
                manifest.node_runtime.file_count
                    + manifest.host_runtime.file_count
                    + manifest.builtin_plugins.file_count,
                manifest.node_runtime.unpacked_size
                    + manifest.host_runtime.unpacked_size
                    + manifest.builtin_plugins.unpacked_size
            );
            Ok(())
        }
        _ => Err(format!("unsupported payload-tool subcommand: {command}")),
    }
}

/// 生成三个确定性 ZIP 和固定字段 manifest。
fn package(options: &BTreeMap<String, String>) -> Result<(), String> {
    let node_source = required_path(options, "node")?;
    let host_source = required_path(options, "host")?;
    let plugin_source = required_path(options, "plugins")?;
    let output = required_path(options, "output")?;
    let desktop_version = required(options, "desktop-version")?;
    let node_version = required(options, "node-version")?;
    let pnpm_version = required(options, "pnpm-version")?;
    let runtime_abi = required(options, "runtime-abi")?
        .parse::<u32>()
        .map_err(|error| format!("invalid runtime ABI: {error}"))?;
    if runtime_abi == 0 {
        return Err("runtime ABI must be positive".to_owned());
    }
    std::fs::create_dir_all(&output)
        .map_err(|error| format!("could not create {}: {error}", output.display()))?;
    let node_runtime = create_deterministic_zip(
        &node_source,
        &output.join("node-runtime.zip"),
        "node-runtime.zip",
    )?;
    let host_runtime = create_deterministic_zip(
        &host_source,
        &output.join("host-runtime.zip"),
        "host-runtime.zip",
    )?;
    let builtin_plugins = create_deterministic_zip(
        &plugin_source,
        &output.join("builtin-plugins.zip"),
        "builtin-plugins.zip",
    )?;
    let mut manifest = PayloadManifest {
        schema_version: PAYLOAD_SCHEMA_VERSION,
        runtime_abi,
        desktop_version,
        payload_digest: String::new(),
        node_version,
        pnpm_version,
        entries: PayloadEntries {
            node: "node/node.exe".to_owned(),
            host: "host/node_modules/@deepseek-ai/dsh/lib/bin.js".to_owned(),
            plugins: "plugins/node_modules".to_owned(),
        },
        node_runtime,
        host_runtime,
        builtin_plugins,
    };
    manifest.payload_digest = calculate_payload_digest(&manifest)?;
    let mut bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("could not serialize payload manifest: {error}"))?;
    bytes.push(b'\n');
    std::fs::write(output.join("payload-manifest.json"), bytes)
        .map_err(|error| format!("could not write payload manifest: {error}"))?;
    verify_payload(&output, &[runtime_abi])?;
    println!("packaged payload {}", manifest.payload_digest);
    Ok(())
}

/// 将 `--name value` 参数解析为有序映射，拒绝重复和位置参数。
fn parse_options(arguments: &[String]) -> Result<BTreeMap<String, String>, String> {
    let mut options = BTreeMap::new();
    let mut index = 0;
    while index < arguments.len() {
        let name = arguments[index]
            .strip_prefix("--")
            .filter(|name| !name.is_empty())
            .ok_or_else(|| format!("expected --name, got {}", arguments[index]))?;
        index += 1;
        let value = arguments
            .get(index)
            .filter(|value| !value.is_empty() && !value.starts_with("--"))
            .ok_or_else(|| format!("--{name} requires a value"))?;
        if options.insert(name.to_owned(), value.to_owned()).is_some() {
            return Err(format!("duplicate option: --{name}"));
        }
        index += 1;
    }
    Ok(options)
}

/// 读取必填字符串参数。
fn required(options: &BTreeMap<String, String>, name: &str) -> Result<String, String> {
    options
        .get(name)
        .cloned()
        .ok_or_else(|| format!("missing required option --{name}"))
}

/// 读取必填路径参数。
fn required_path(options: &BTreeMap<String, String>, name: &str) -> Result<PathBuf, String> {
    required(options, name).map(PathBuf::from)
}
