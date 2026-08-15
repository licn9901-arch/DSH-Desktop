//! 主窗口、系统托盘和显式退出行为。

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Manager, WebviewWindow, WindowEvent};
use tauri_plugin_dialog::DialogExt;

use crate::logger::{log_app, log_file_path};

const MENU_OPEN: &str = "open-main";
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

/// 创建带打开、日志、关于和退出命令的原生托盘。
pub fn create_tray(app: &App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, MENU_OPEN, "打开主窗口", true, None::<&str>)?;
    let log = MenuItem::with_id(app, MENU_LOG, "打开日志", true, None::<&str>)?;
    let about = MenuItem::with_id(app, MENU_ABOUT, "关于", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &log, &about, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main-tray")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("DeepSeek Harness Desktop")
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_OPEN => show_main_window(app),
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
            "DeepSeek Harness Desktop {}\n\n社区项目，非 DeepSeek 官方产品。\n\n内置 DSH 0.1.0-rc.6\n插件：At File 0.6.0、GenUI 0.8.4、Better Sidebar 0.12.2、Skins 0.1.16\n\n侧栏可访问文件、Git 和 PTY；皮肤中心可写 DSH 配置；GenUI action 会回传模型。",
            app.package_info().version
        ))
        .title("关于 DeepSeek Harness Desktop")
        .blocking_show();
}
