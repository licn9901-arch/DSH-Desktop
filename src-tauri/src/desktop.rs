//! 主窗口、系统托盘和显式退出行为。

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Manager, WebviewWindow, WindowEvent};
use tauri_plugin_dialog::DialogExt;

use crate::lifecycle::HostController;
use crate::logger::{log_app, log_error, log_file_path};

const MENU_OPEN: &str = "open-main";
const MENU_RESTART: &str = "restart-host";
const MENU_LOG: &str = "open-log";
const MENU_ABOUT: &str = "about";
const MENU_QUIT: &str = "quit";

/// 保存用户是否已经选择显式退出，区分隐藏窗口与结束应用。
#[derive(Default)]
pub struct DesktopLifecycle {
    quitting: AtomicBool,
}

impl DesktopLifecycle {
    /// 标记显式退出；重复调用保持幂等。
    pub fn request_quit(&self) {
        self.quitting.store(true, Ordering::SeqCst);
    }

    /// 返回当前是否正在显式退出。
    pub fn is_quitting(&self) -> bool {
        self.quitting.load(Ordering::SeqCst)
    }
}

/// 为主窗口注册关闭到托盘行为。
pub fn configure_close_to_tray(window: &WebviewWindow) {
    let window_for_event = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            let lifecycle = window_for_event.state::<DesktopLifecycle>();
            if !lifecycle.is_quitting() {
                api.prevent_close();
                let _ = window_for_event.hide();
                log_app("main window hidden to tray");
            }
        }
    });
}

/// 创建带打开、重启 Host、日志、关于和退出命令的原生托盘。
pub fn create_tray(app: &App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, MENU_OPEN, "打开主窗口", true, None::<&str>)?;
    let restart = MenuItem::with_id(app, MENU_RESTART, "重启 DSH 服务", true, None::<&str>)?;
    let log = MenuItem::with_id(app, MENU_LOG, "打开日志", true, None::<&str>)?;
    let about = MenuItem::with_id(app, MENU_ABOUT, "关于", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &restart, &log, &about, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main-tray")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("DeepSeek Harness Desktop")
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_OPEN => show_main_window(app),
            MENU_RESTART => request_host_restart(app),
            MENU_LOG => open_log_file(),
            MENU_ABOUT => show_about(app),
            MENU_QUIT => quit_application(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// 将托盘重启请求交给桌面 Host 控制器，状态冲突时向用户给出明确提示。
fn request_host_restart(app: &AppHandle) {
    let Some(controller) = app.try_state::<HostController>() else {
        log_error("host controller is unavailable");
        return;
    };
    match controller.restart() {
        Ok(()) => log_app("host restart requested from tray"),
        Err(message) => {
            log_error(&format!("host restart request rejected: {message}"));
            let _ = app
                .dialog()
                .message("DSH 服务当前正在启动、重启或退出，请稍后再试。")
                .title("无法重启 DSH 服务")
                .blocking_show();
        }
    }
}

/// 恢复主窗口并将输入焦点交给它，供托盘和二次启动复用。
pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// 标记显式退出并触发 Tauri 退出流程，Host 由统一退出回调清理。
pub fn quit_application(app: &AppHandle) {
    app.state::<DesktopLifecycle>().request_quit();
    log_app("explicit application exit requested");
    app.exit(0);
}

/// 把外部 HTTP/HTTPS 地址交给 Windows 默认浏览器，不在 WebView 内加载。
pub fn open_external_url(url: &url::Url) {
    let _ = Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", url.as_str()])
        .spawn();
}

/// 使用资源管理器定位日志文件，避免把日志内容暴露给 WebView。
fn open_log_file() {
    let path = log_file_path();
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path);
    let argument = format!("/select,{}", path.display());
    let _ = Command::new("explorer.exe").arg(argument).spawn();
}

/// 显示版本与非官方声明，不向远程页面暴露任何命令。
fn show_about(app: &AppHandle) {
    let _ = app
        .dialog()
        .message(format!(
            "DeepSeek Harness Desktop {}\n\n社区项目，非 DeepSeek 官方产品。\n\n内置 DSH 0.1.0-rc.6、DSH Market 1.9.0、pnpm 11.22.0\n插件：At File 0.6.0、GenUI 0.8.4、Better Sidebar 0.12.2、Skins 0.1.17、Hindsight 0.3.4、ModLens 3.16.7、Skills/MCP 0.2.3\n\n第三方插件与桌面应用拥有相同主机权限，目前没有签名验证、权限清单或进程级沙箱。",
            app.package_info().version
        ))
        .title("关于 DeepSeek Harness Desktop")
        .blocking_show();
}
