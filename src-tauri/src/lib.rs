//! DeepSeek Harness Desktop 的桌面封装与生命周期入口。

pub mod host;
pub mod lifecycle;
pub mod logger;
pub mod readiness;
pub mod runtime;

use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use host::HostSupervisor;
use lifecycle::HostEvent;
use logger::{log_app, log_host};
use readiness::ReadinessParser;
use runtime::RuntimePaths;
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;

static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

/// 构建并运行桌面应用，负责窗口、Host 和退出清理的顶层编排。
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(setup_application)
        .build(tauri::generate_context!())
        .expect("error while building the tauri application")
        .run(|app_handle, event| {
            if let RunEvent::Exit = event {
                SHUTTING_DOWN.store(true, Ordering::SeqCst);
                if let Some(supervisor) = app_handle.try_state::<HostSupervisor>() {
                    supervisor.shutdown();
                }
            }
        });
}

/// 创建启动页、启动唯一 Host，并注册读取与监视线程。
fn setup_application(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("DeepSeek Harness Desktop")
        .inner_size(1280.0, 800.0)
        .min_inner_size(960.0, 600.0)
        .build()?;

    let resource_dir = app.path().resource_dir().unwrap_or_default();
    let runtime = match RuntimePaths::resolve(&resource_dir) {
        Ok(runtime) => runtime,
        Err(message) => {
            fail(&handle, &message);
            return Ok(());
        }
    };
    let readiness_timeout = runtime.readiness_timeout;
    let (supervisor, stdout, stderr) = match HostSupervisor::spawn(&runtime) {
        Ok(parts) => parts,
        Err(message) => {
            fail(&handle, &message);
            return Ok(());
        }
    };
    app.manage(supervisor);

    let (sender, receiver) = mpsc::channel::<HostEvent>();
    spawn_stdout_reader(stdout, sender.clone());
    spawn_stderr_reader(stderr);
    spawn_exit_watcher(handle.clone(), sender);
    spawn_boot_coordinator(handle, receiver, readiness_timeout);
    Ok(())
}

/// 持续排空 Host stdout，并把第一条就绪地址发送给启动协调线程。
fn spawn_stdout_reader(stdout: std::process::ChildStdout, sender: mpsc::Sender<HostEvent>) {
    std::thread::spawn(move || {
        let mut parser = ReadinessParser::new();
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else {
                break;
            };
            log_host(&line);
            if let Some(url) = parser.parse_line(&line) {
                let _ = sender.send(HostEvent::Ready(url));
            }
        }
        if !parser.is_ready() {
            let _ = sender.send(HostEvent::Exited);
        }
    });
}

/// 持续排空 Host stderr，防止管道塞满后阻塞子进程。
fn spawn_stderr_reader(stderr: std::process::ChildStderr) {
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            log_host(&line);
        }
    });
}

/// 轮询 Host 退出状态，并在结束时通知启动协调线程。
fn spawn_exit_watcher(handle: AppHandle, sender: mpsc::Sender<HostEvent>) {
    std::thread::spawn(move || loop {
        if handle.state::<HostSupervisor>().has_exited() {
            let _ = sender.send(HostEvent::Exited);
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    });
}

/// 等待 Host 首次就绪、导航主窗口，并持续处理后续异常退出。
fn spawn_boot_coordinator(
    handle: AppHandle,
    receiver: mpsc::Receiver<HostEvent>,
    readiness_timeout: Duration,
) {
    std::thread::spawn(move || {
        let boot_started = Instant::now();
        let ready_url = match receiver.recv_timeout(readiness_timeout) {
            Ok(HostEvent::Ready(url)) => url,
            Ok(HostEvent::Exited) => {
                if !SHUTTING_DOWN.load(Ordering::SeqCst) {
                    fail(
                        &handle,
                        "DeepSeek Harness exited before becoming ready. Check the host log for errors.",
                    );
                }
                return;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                fail(
                    &handle,
                    &format!(
                        "DeepSeek Harness did not report a URL within {} seconds. Check the host log for errors.",
                        readiness_timeout.as_secs()
                    ),
                );
                return;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
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

        while let Ok(event) = receiver.recv() {
            match event {
                HostEvent::Exited if !SHUTTING_DOWN.load(Ordering::SeqCst) => {
                    handle.exit(0);
                    return;
                }
                HostEvent::Exited | HostEvent::Ready(_) => {}
            }
        }
    });
}

/// 记录启动错误、显示诊断提示并以失败状态退出应用。
fn fail(handle: &AppHandle, message: &str) {
    log_app(message);
    let _ = handle
        .dialog()
        .message(format!("DeepSeek Harness failed to start.\n\n{message}"))
        .title("DeepSeek Harness Desktop")
        .blocking_show();
    handle.exit(1);
}
