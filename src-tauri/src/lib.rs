//! DeepSeek Harness Desktop 的桌面封装与生命周期入口。

pub mod desktop;
pub mod host;
pub mod lifecycle;
pub mod logger;
pub mod navigation;
pub mod plugins;
pub mod readiness;
pub mod runtime;
pub mod sidebar_settings;

use std::io::{BufRead, BufReader};
use std::sync::mpsc;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use desktop::{
    configure_close_to_tray, create_tray, open_external_url, quit_application, show_main_window,
};
use host::HostSupervisor;
use lifecycle::{HostEvent, LifecycleAction, LifecycleStateMachine};
use logger::{log_app, log_error, log_file_path, log_host};
use navigation::{decide_navigation, NavigationDecision};
use plugins::{PluginManager, PluginTransaction};
use readiness::ReadinessParser;
use runtime::RuntimePaths;
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;

/// 构建并运行桌面应用，负责窗口、Host 和退出清理的顶层编排。
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(
            |app, arguments, _cwd| {
                if arguments
                    .iter()
                    .any(|argument| argument == "--quit-existing")
                {
                    log_app("secondary launch requested explicit exit");
                    quit_application(app);
                    return;
                }
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
    let mut plugin_degraded_reason = None;
    let mut plugin_transaction = match PluginManager::new(&runtime).prepare() {
        Ok(transaction) => Some(transaction),
        Err(message) => {
            log_error(&format!(
                "managed plugins were disabled before host startup: {message}"
            ));
            plugin_degraded_reason = Some(message);
            None
        }
    };

    app.manage(HostSupervisor::new());
    let receiver = match start_host_streams(&handle, &runtime) {
        Ok(receiver) => receiver,
        Err(plugin_error) if plugin_transaction.is_some() => {
            log_error(&format!(
                "host failed with managed plugins; rolling back before one core retry: {plugin_error}"
            ));
            plugin_degraded_reason = Some(plugin_error.clone());
            if let Some(transaction) = plugin_transaction.take() {
                if let Err(rollback) = transaction.rollback() {
                    fail(
                        &handle,
                        &format!("{plugin_error}; plugin rollback failed: {rollback}"),
                    );
                    return Ok(());
                }
            }
            match start_host_streams(&handle, &runtime) {
                Ok(receiver) => receiver,
                Err(message) => {
                    fail(&handle, &message);
                    return Ok(());
                }
            }
        }
        Err(message) => {
            fail(&handle, &message);
            return Ok(());
        }
    };
    spawn_boot_coordinator(
        handle,
        receiver,
        runtime,
        host_origin,
        plugin_transaction,
        plugin_degraded_reason,
    );
    Ok(())
}

/// 启动当前 supervisor 中的 Host，并为本次 PID 创建独立事件通道。
fn start_host_streams(
    handle: &AppHandle,
    runtime: &RuntimePaths,
) -> Result<mpsc::Receiver<HostEvent>, String> {
    let supervisor = handle.state::<HostSupervisor>();
    let (stdout, stderr) = supervisor.start(runtime)?;
    let pid = supervisor
        .pid()
        .ok_or_else(|| "host PID is not available after startup".to_owned())?;
    let (sender, receiver) = mpsc::channel::<HostEvent>();
    spawn_stdout_reader(stdout, sender.clone());
    spawn_stderr_reader(stderr);
    spawn_exit_watcher(handle.clone(), sender, pid);
    Ok(receiver)
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
fn spawn_exit_watcher(handle: AppHandle, sender: mpsc::Sender<HostEvent>, expected_pid: u32) {
    std::thread::spawn(move || loop {
        let supervisor = handle.state::<HostSupervisor>();
        if supervisor.pid() != Some(expected_pid) {
            return;
        }
        if let Some(exit_code) = supervisor.try_exit_code() {
            let _ = sender.send(HostEvent::Exited(exit_code));
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    });
}

/// 等待 Host 首次就绪、导航主窗口，并持续处理后续异常退出。
fn spawn_boot_coordinator(
    handle: AppHandle,
    initial_receiver: mpsc::Receiver<HostEvent>,
    runtime: RuntimePaths,
    host_origin: Arc<RwLock<Option<url::Url>>>,
    mut plugin_transaction: Option<PluginTransaction>,
    mut plugin_degraded_reason: Option<String>,
) {
    std::thread::spawn(move || {
        let boot_started = Instant::now();
        let mut receiver = initial_receiver;
        let (mut lifecycle, mut ready_url) = match await_host_ready(
            &handle,
            &receiver,
            runtime.readiness_timeout,
        ) {
            Ok(ready) => ready,
            Err(plugin_error) if plugin_transaction.is_some() => {
                log_error(&format!(
                    "managed plugin startup failed; restoring profile and retrying core once: {plugin_error}"
                ));
                plugin_degraded_reason = Some(plugin_error.clone());
                handle.state::<HostSupervisor>().shutdown();
                if let Some(transaction) = plugin_transaction.take() {
                    if let Err(rollback) = transaction.rollback() {
                        fail(
                            &handle,
                            &format!("{plugin_error}; plugin rollback failed: {rollback}"),
                        );
                        return;
                    }
                }
                receiver = match start_host_streams(&handle, &runtime) {
                    Ok(receiver) => receiver,
                    Err(core_error) => {
                        fail(&handle, &core_error);
                        return;
                    }
                };
                match await_host_ready(&handle, &receiver, runtime.readiness_timeout) {
                    Ok(ready) => ready,
                    Err(core_error) => {
                        fail(&handle, &core_error);
                        return;
                    }
                }
            }
            Err(message) => {
                fail(&handle, &message);
                return;
            }
        };

        if plugin_transaction
            .as_ref()
            .is_some_and(PluginTransaction::should_seed_sidebar)
        {
            let seed_result = ready_url
                .parse::<url::Url>()
                .map_err(|error| format!("invalid ready URL for sidebar settings: {error}"))
                .and_then(|origin| sidebar_settings::initialize_sidebar_defaults(&origin));
            match seed_result {
                Ok(()) => {
                    if let Some(transaction) = plugin_transaction.as_mut() {
                        transaction.mark_sidebar_seeded();
                    }
                }
                Err(plugin_error) => {
                    log_error(&format!(
                        "Better Sidebar security initialization failed; retrying core without managed plugins: {plugin_error}"
                    ));
                    plugin_degraded_reason = Some(plugin_error.clone());
                    handle.state::<HostSupervisor>().shutdown();
                    if let Some(transaction) = plugin_transaction.take() {
                        if let Err(rollback) = transaction.rollback() {
                            fail(
                                &handle,
                                &format!("{plugin_error}; plugin rollback failed: {rollback}"),
                            );
                            return;
                        }
                    }
                    receiver = match start_host_streams(&handle, &runtime) {
                        Ok(receiver) => receiver,
                        Err(core_error) => {
                            fail(&handle, &core_error);
                            return;
                        }
                    };
                    (lifecycle, ready_url) =
                        match await_host_ready(&handle, &receiver, runtime.readiness_timeout) {
                            Ok(ready) => ready,
                            Err(core_error) => {
                                fail(&handle, &core_error);
                                return;
                            }
                        };
                }
            }
        }

        if let Some(transaction) = plugin_transaction.take() {
            if let Err(message) = transaction.commit() {
                handle.state::<HostSupervisor>().shutdown();
                fail(
                    &handle,
                    &format!("failed to commit managed plugins: {message}"),
                );
                return;
            }
        }

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
        if let Some(reason) = plugin_degraded_reason {
            let _ = handle
                .dialog()
                .message(format!(
                    "内置插件本次未启用，DSH 核心已降级启动。\n\n{reason}\n\n日志：{}",
                    log_file_path().display()
                ))
                .title("DeepSeek Harness Desktop 插件降级")
                .blocking_show();
        }

        while let Ok(event) = receiver.recv() {
            let shutting_down = handle.state::<desktop::DesktopLifecycle>().is_quitting();
            match lifecycle.on_event(event, shutting_down) {
                LifecycleAction::Fail { message, .. } => {
                    fail(&handle, &message);
                    return;
                }
                LifecycleAction::Ignore | LifecycleAction::Navigate(_) => {}
            }
        }
    });
}

/// 等待一个 Host 实例首次就绪，并返回后续事件需要复用的状态机。
fn await_host_ready(
    handle: &AppHandle,
    receiver: &mpsc::Receiver<HostEvent>,
    readiness_timeout: Duration,
) -> Result<(LifecycleStateMachine, String), String> {
    let mut lifecycle = LifecycleStateMachine::new();
    let shutting_down = handle.state::<desktop::DesktopLifecycle>().is_quitting();
    let action = match receiver.recv_timeout(readiness_timeout) {
        Ok(event) => lifecycle.on_event(event, shutting_down),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            lifecycle.on_timeout(shutting_down, readiness_timeout.as_secs())
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            return Err("host event channel disconnected before readiness".to_owned())
        }
    };
    match action {
        LifecycleAction::Navigate(url) => Ok((lifecycle, url)),
        LifecycleAction::Fail { message, .. } => Err(message),
        LifecycleAction::Ignore => Err("host startup was cancelled".to_owned()),
    }
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
