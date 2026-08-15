//! DeepSeek Harness Desktop — a Tauri shell around the DSH web host.
//!
//! The DSH UI is fundamentally a local web application: `dsh web` serves it
//! over loopback HTTP. This shell (mirroring the official Electron shell in
//! `resources/app.asar`) spawns that host process, waits for its readiness
//! URL on stdout (`dsh web: http://127.0.0.1:<port>`), then loads the URL in
//! a WebView2 window. Closing the window terminates the host process.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::env;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;

/// `CREATE_NO_WINDOW` keeps console children (node.exe, taskkill.exe) from
/// flashing/opening a black console window when the GUI app launches them.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Prevent console helper processes from creating a visible console window.
#[cfg(windows)]
fn hide_console_window(command: &mut Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console_window(_command: &mut Command) {}

/// Default timeout when waiting for the host to print its readiness URL.
/// Can be overridden at runtime with `DSH_DESKTOP_READY_TIMEOUT_SECS`.
const READINESS_TIMEOUT: Duration = Duration::from_secs(90);

/// Set once the app is intentionally shutting down, so late host-exit events
/// do not pop error dialogs over a window the user already closed.
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

/// Events produced by the host child process.
enum HostEvent {
    Ready(String),
    Exited,
}

/// Owns the host child process so the app can tear it down on exit.
struct HostState(Mutex<Option<Child>>);

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let handle = app.handle().clone();

            // Show the placeholder window right away; navigate to the host
            // URL once the server reports readiness.
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("DeepSeek Harness Desktop")
                .inner_size(1280.0, 800.0)
                .min_inner_size(960.0, 600.0)
                .build()
                .expect("failed to create main window");

            let resource_dir = app.path().resource_dir().unwrap_or_default();
            let mut child = match spawn_host(&resource_dir) {
                Ok(child) => child,
                Err(message) => {
                    fail(&handle, &message);
                    return Ok(());
                }
            };
            let stdout = child.stdout.take().expect("host stdout must be piped");
            let stderr = child.stderr.take().expect("host stderr must be piped");
            app.manage(HostState(Mutex::new(Some(child))));

            let (tx, rx) = mpsc::channel::<HostEvent>();

            // Reader 1: host stdout. Watch for the readiness URL, then keep
            // draining so the pipe can never fill up and stall the host.
            {
                let tx = tx.clone();
                std::thread::spawn(move || {
                    let reader = BufReader::new(stdout);
                    let mut ready = false;
                    for line in reader.lines() {
                        let line = match line {
                            Ok(line) => line,
                            Err(_) => break,
                        };
                        log_host(&line);
                        if !ready {
                            if let Some(url) = extract_loopback_url(&line) {
                                ready = true;
                                let _ = tx.send(HostEvent::Ready(url));
                            }
                        }
                    }
                    // EOF before readiness: the host exited or crashed.
                    if !ready {
                        let _ = tx.send(HostEvent::Exited);
                    }
                });
            }

            // Reader 2: host stderr. Drain into the log file.
            {
                std::thread::spawn(move || {
                    let reader = BufReader::new(stderr);
                    for line in reader.lines().map_while(Result::ok) {
                        log_host(&line);
                    }
                });
            }

            // Watcher: detect unexpected host exit while the app is running.
            {
                let tx = tx.clone();
                let handle_watcher = handle.clone();
                std::thread::spawn(move || loop {
                    let exited = {
                        let state = handle_watcher.state::<HostState>();
                        let mut guard = state.0.lock().unwrap();
                        match guard.as_mut() {
                            Some(child) => child.try_wait().map(|s| s.is_some()).unwrap_or(true),
                            None => true,
                        }
                    };
                    if exited {
                        let _ = tx.send(HostEvent::Exited);
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(500));
                });
            }

            // Boot thread: wait for readiness, navigate, and react to a host
            // that dies before or after readiness.
            std::thread::spawn(move || {
                let boot_started = Instant::now();
                let readiness_timeout = env::var("DSH_DESKTOP_READY_TIMEOUT_SECS")
                    .ok()
                    .and_then(|value| value.trim().parse::<u64>().ok())
                    .map(Duration::from_secs)
                    .unwrap_or(READINESS_TIMEOUT);
                let deadline = boot_started + readiness_timeout;

                let ready_url = loop {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        fail(
                            &handle,
                            &format!(
                                "DeepSeek Harness did not report a URL within {} seconds. Check the host log for errors.",
                                readiness_timeout.as_secs()
                            ),
                        );
                        return;
                    }
                    match rx.recv_timeout(remaining) {
                        Ok(HostEvent::Ready(url)) => break Some(url),
                        Ok(HostEvent::Exited) => {
                            if SHUTTING_DOWN.load(Ordering::SeqCst) {
                                return;
                            }
                            fail(
                                &handle,
                                "DeepSeek Harness exited before becoming ready. Check the host log for errors.",
                            );
                            return;
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            fail(
                                &handle,
                                "Timed out waiting for DeepSeek Harness to start. Check the host log for errors.",
                            );
                            return;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                };

                let ready_url = match ready_url {
                    Some(url) => url,
                    None => return,
                };

                log_app(&format!(
                    "host ready: {ready_url} (started in {} ms)",
                    boot_started.elapsed().as_millis()
                ));
                if let Ok(parsed) = ready_url.parse::<url::Url>() {
                    if let Some(window) = handle.get_webview_window("main") {
                        let _ = window.navigate(parsed);
                    }
                }

                // Readiness has been reached. The remaining wait is
                // intentionally unbounded: the watcher thread sends
                // `Exited` if the host process ever goes away.
                loop {
                    match rx.recv() {
                        Ok(HostEvent::Exited) => {
                            if SHUTTING_DOWN.load(Ordering::SeqCst) {
                                return;
                            }
                            // Host died after readiness; nothing left to show.
                            handle.exit(0);
                            return;
                        }
                        Ok(HostEvent::Ready(_)) => {
                            // Late duplicate readiness line; keep waiting.
                        }
                        Err(_) => return,
                    }
                }
            });

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building the tauri application")
        .run(|app_handle, event| {
            if let RunEvent::Exit = event {
                SHUTTING_DOWN.store(true, Ordering::SeqCst);
                if let Some(state) = app_handle.try_state::<HostState>() {
                    if let Some(child) = state.0.lock().unwrap().take() {
                        // Ask the process tree to close politely first, then
                        // force-kill. DSH persists state incrementally, so a
                        // hard kill is an acceptable fallback.
                        let pid: String = child.id().to_string();
                        let pid_ref: &str = &pid;
                        let mut killer = Command::new("taskkill");
                        killer.args(["/PID", pid_ref, "/T"]);
                        hide_console_window(&mut killer);
                        let _ = killer.status();
                        std::thread::sleep(Duration::from_millis(1200));
                        let mut child = child;
                        let _ = child.kill();
                        let _ = child.wait();
                    }
                }
            }
        });
}

/// Spawn the DSH host exactly like the official desktop shell does:
/// `node --expose-internals <dsh>/lib/bin.js web --host 127.0.0.1 --port 0`.
fn spawn_host(resource_dir: &Path) -> Result<Child, String> {
    let node = resolve_node(resource_dir);
    let entry = resolve_cli_entry(resource_dir)?;
    let entry_ref: &str = &entry;
    let cwd = env::var("DSH_DESKTOP_CWD").unwrap_or_else(|_| {
        env::var("USERPROFILE")
            .or_else(|_| env::var("HOME"))
            .unwrap_or_else(|_| ".".into())
    });

    log_app(&format!(
        "spawning: {node} --expose-internals {entry} web --host 127.0.0.1 --port 0 (cwd: {cwd})"
    ));

    let mut host = Command::new(&node);
    host.args([
        "--expose-internals",
        entry_ref,
        "web",
        "--host",
        "127.0.0.1",
        "--port",
        "0",
    ])
    .current_dir(&cwd)
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    hide_console_window(&mut host);
    host.spawn()
        .map_err(|e| format!("failed to start Node ({node}): {e}"))
}

/// Resolve the node executable: env override, bundled copy, PATH, in order.
fn resolve_node(resource_dir: &Path) -> String {
    if let Ok(v) = env::var("DSH_DESKTOP_NODE_EXECUTABLE") {
        if !v.is_empty() {
            return v;
        }
    }
    let bundled = resource_dir.join("node/node.exe");
    if bundled.is_file() {
        return bundled.to_string_lossy().into_owned();
    }
    if let Some(found) = find_in_path("node.exe") {
        return found;
    }
    "node".into()
}

/// Resolve the dsh CLI entry (`lib/bin.js`), in order:
/// 1. `DSH_DESKTOP_CLI_ENTRY` env override
/// 2. bundled host runtime next to the app (for self-contained distribution)
/// 3. the official desktop install's host runtime
/// 4. the globally installed npm `dsh` package
fn resolve_cli_entry(resource_dir: &Path) -> Result<String, String> {
    const REL: &str = "node_modules/@deepseek-ai/dsh/lib/bin.js";

    if let Ok(v) = env::var("DSH_DESKTOP_CLI_ENTRY") {
        if !v.is_empty() {
            if Path::new(&v).is_file() {
                return Ok(v);
            }
            return Err(format!(
                "DSH_DESKTOP_CLI_ENTRY is set but the file does not exist: {v}"
            ));
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    candidates.push(resource_dir.join("host").join(REL)); // bundled host
    candidates.push(
        PathBuf::from(r"C:\Program Files\DeepSeek Harness\resources\host").join(REL),
    );
    if let Some(node) = find_in_path("node.exe") {
        if let Some(dir) = Path::new(&node).parent() {
            candidates.push(dir.join("node_modules/@deepseek-ai/dsh/lib/bin.js"));
        }
    }

    for candidate in &candidates {
        if candidate.is_file() {
            return Ok(candidate.to_string_lossy().into_owned());
        }
    }

    Err(format!(
        "could not locate the dsh CLI entry (looked in: {}). \
         Set DSH_DESKTOP_CLI_ENTRY to the path of @deepseek-ai/dsh/lib/bin.js.",
        candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("; ")
    ))
}

/// Pull the loopback HTTP URL out of a host stdout line. The host prints
/// `dsh web: http://127.0.0.1:<port>` (optionally followed by a LAN hint).
/// Validation mirrors the official desktop shell's readiness check.
fn extract_loopback_url(line: &str) -> Option<String> {
    let start = line.find("http://")?;
    let rest = &line[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let token = &rest[..end];
    let parsed = url::Url::parse(token).ok()?;
    if parsed.scheme() != "http" {
        return None;
    }
    let host = parsed.host_str()?;
    if host != "127.0.0.1" && host != "localhost" {
        return None;
    }
    let port = parsed.port()?;
    if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
        return None;
    }
    Some(format!("http://{host}:{port}"))
}

/// Search PATH for an executable by name.
fn find_in_path(name: &str) -> Option<String> {
    let path = env::var_os("PATH")?;
    for dir in env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

/// Show an error dialog and quit.
fn fail(handle: &AppHandle, message: &str) {
    log_app(message);
    let _ = handle
        .dialog()
        .message(format!("DeepSeek Harness failed to start.\n\n{message}"))
        .title("DeepSeek Harness Desktop")
        .blocking_show();
    handle.exit(1);
}

fn log_file_path() -> PathBuf {
    let base = env::var("LOCALAPPDATA")
        .or_else(|_| env::var("TEMP"))
        .unwrap_or_else(|_| ".".into());
    let dir = Path::new(&base).join("dsh-desktop");
    let _ = fs::create_dir_all(&dir);
    dir.join("dsh-desktop.log")
}

fn log_app(message: &str) {
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file_path())
    {
        let _ = writeln!(file, "[app] {message}");
    }
}

fn log_host(line: &str) {
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file_path())
    {
        let _ = writeln!(file, "[host] {line}");
    }
}
