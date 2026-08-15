//! DeepSeek Harness Desktop 的桌面封装与生命周期入口。

pub mod desktop;
pub mod host;
pub mod lifecycle;
pub mod logger;
pub mod navigation;
pub mod readiness;
pub mod runtime;

use std::io::{BufRead, BufReader};
use std::sync::mpsc;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use desktop::{configure_close_to_tray, create_tray, open_external_url, show_main_window};
use host::HostSupervisor;
use lifecycle::HostEvent;
use logger::{log_app, log_error, log_file_path, log_host};
use navigation::{decide_navigation, NavigationDecision};
use readiness::ReadinessParser;
use runtime::RuntimePaths;
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;

/// 构建并运行桌面应用，负责窗口、Host 和退出清理的顶层编排。
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(
            |app, _arguments, _cwd| {
                log_app("secondary launch requested; focusing existing window");
                show_main_window(app);
            },
        ))
        .plugin(tauri_plugin_dialog::init())
        .setup(setup_application)
        .build(tauri::generate_context!())
        .expect("error while building the tauri application")
        .run(|app_handle, event| {
            if let RunEvent::Exit = event {
                app_handle
                    .state::<desktop::DesktopLifecycle>()
                    .request_quit();
                if let Some(supervisor) = app_handle.try_state::<HostSupervisor>() {
                    supervisor.shutdown();
                }
            }
        });
}

/// 创建启动页、启动唯一 Host，并注册读取与监视线程。
fn setup_application(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    app.manage(desktop::DesktopLifecycle::default());
    let host_origin = Arc::new(RwLock::new(None::<url::Url>));
    let navigation_origin = host_origin.clone();
    let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("DeepSeek Harness Desktop")
        .inner_size(1280.0, 800.0)
        .min_inner_size(960.0, 600.0)
        .on_navigation(move |target| {
            let origin = navigation_origin
                .read()
                .unwrap_or_else(|error| error.into_inner());
            match decide_navigation(origin.as_ref(), target) {
                NavigationDecision::Allow => true,
                NavigationDecision::OpenExternal => {
                    open_external_url(target);
                    false
                }
                NavigationDecision::Deny => {
                    log_error(&format!(
                        "blocked WebView navigation: scheme={}",
                        target.scheme()
                    ));
                    false
                }
            }
        })
        .build()?;
    configure_close_to_tray(&window);
    create_tray(app)?;

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
    spawn_boot_coordinator(handle, receiver, readiness_timeout, host_origin);
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
            match parser.parse_line(&line) {
                Ok(Some(url)) => {
                    let _ = sender.send(HostEvent::Ready(url));
                }
                Ok(None) => {}
                Err(error) => {
                    let _ = sender.send(HostEvent::ProtocolError(error.to_string()));
                    break;
                }
            }
        }
        if !parser.is_ready() {
            let _ = sender.send(HostEvent::Exited(None));
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
        if let Some(exit_code) = handle.state::<HostSupervisor>().try_exit_code() {
            let _ = sender.send(HostEvent::Exited(exit_code));
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
    host_origin: Arc<RwLock<Option<url::Url>>>,
) {
    std::thread::spawn(move || {
        let boot_started = Instant::now();
        let ready_url = match receiver.recv_timeout(readiness_timeout) {
            Ok(HostEvent::Ready(url)) => url,
            Ok(HostEvent::Exited(exit_code)) => {
                if !handle.state::<desktop::DesktopLifecycle>().is_quitting() {
                    fail(
                        &handle,
                        &format!(
                            "DeepSeek Harness exited before becoming ready (exit code: {exit_code:?})."
                        ),
                    );
                }
                return;
            }
            Ok(HostEvent::ProtocolError(message)) => {
                fail(
                    &handle,
                    &format!("DeepSeek Harness readiness protocol error: {message}"),
                );
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
            *host_origin
                .write()
                .unwrap_or_else(|error| error.into_inner()) = Some(parsed.clone());
            if let Some(window) = handle.get_webview_window("main") {
                let _ = window.navigate(parsed);
            }
        }

        while let Ok(event) = receiver.recv() {
            match event {
                HostEvent::Exited(exit_code)
                    if !handle.state::<desktop::DesktopLifecycle>().is_quitting() =>
                {
                    fail(
                        &handle,
                        &format!(
                            "DeepSeek Harness exited unexpectedly (exit code: {exit_code:?})."
                        ),
                    );
                    return;
                }
                HostEvent::ProtocolError(message) => {
                    fail(
                        &handle,
                        &format!("DeepSeek Harness readiness protocol error: {message}"),
                    );
                    return;
                }
                HostEvent::Exited(_) | HostEvent::Ready(_) => {}
            }
        }
    });
}

/// 记录启动错误、显示诊断提示并以失败状态退出应用。
fn fail(handle: &AppHandle, message: &str) {
    log_error(message);
    let log_path = log_file_path();
    let _ = handle
        .dialog()
        .message(format!(
            "DeepSeek Harness failed.\n\n{message}\n\nLog: {}",
            log_path.display()
        ))
        .title("DeepSeek Harness Desktop")
        .blocking_show();
    handle.exit(1);
}
