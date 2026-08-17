//! WebView 导航安全策略。

/// 导航请求经过安全策略后的处理方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationDecision {
    /// 允许 WebView 在当前窗口内加载。
    Allow,
    /// 阻止 WebView，并交给系统默认浏览器打开。
    OpenExternal,
    /// 阻止且不执行任何外部动作。
    Deny,
}

/// 判断目标是否为 Tauri 在启动阶段加载的内置资源地址。
///
/// Windows WebView2 会把 `WebviewUrl::App` 映射成 `http://tauri.localhost`，
/// 其他平台或运行模式则可能使用 `tauri://localhost`。这里只允许 Tauri
/// 自身使用的精确主机名，避免相似域名或带自定义端口的地址进入启动白名单。
fn is_tauri_startup_url(target: &url::Url) -> bool {
    let valid_origin = matches!(
        (target.scheme(), target.host_str()),
        ("tauri", Some("localhost"))
            | ("http", Some("tauri.localhost"))
            | ("https", Some("tauri.localhost"))
    );

    valid_origin
        && target.port().is_none()
        && target.username().is_empty()
        && target.password().is_none()
}

/// 根据当前 Host 原点判断目标地址如何处理。
pub fn decide_navigation(host_origin: Option<&url::Url>, target: &url::Url) -> NavigationDecision {
    if host_origin.is_none() && is_tauri_startup_url(target) {
        return NavigationDecision::Allow;
    }

    if let Some(origin) = host_origin {
        let same_origin = target.scheme() == origin.scheme()
            && target.host_str() == origin.host_str()
            && target.port_or_known_default() == origin.port_or_known_default();
        if same_origin {
            return NavigationDecision::Allow;
        }
    }

    match target.scheme() {
        "http" | "https" => NavigationDecision::OpenExternal,
        _ => NavigationDecision::Deny,
    }
}

/// 判断新窗口请求如何处理。
///
/// 外部 HTTP(S) 仍交给系统浏览器；即使目标与当前 Host 同源，也不允许插件通过
/// `target=_blank` 或 `window.open` 创建第二个 Harness 桌面窗口。
pub fn decide_new_window(host_origin: Option<&url::Url>, target: &url::Url) -> NavigationDecision {
    match decide_navigation(host_origin, target) {
        NavigationDecision::Allow => NavigationDecision::Deny,
        decision => decision,
    }
}

#[cfg(test)]
mod tests {
    use super::{decide_navigation, decide_new_window, NavigationDecision};

    fn url(value: &str) -> url::Url {
        url::Url::parse(value).unwrap()
    }

    #[test]
    fn permits_local_start_page_and_same_host_origin() {
        let origin = url("http://127.0.0.1:43123/");
        assert_eq!(
            decide_navigation(None, &url("tauri://localhost/index.html")),
            NavigationDecision::Allow
        );
        assert_eq!(
            decide_navigation(None, &url("http://tauri.localhost/")),
            NavigationDecision::Allow
        );
        assert_eq!(
            decide_navigation(None, &url("https://tauri.localhost/index.html")),
            NavigationDecision::Allow
        );
        assert_eq!(
            decide_navigation(Some(&origin), &url("http://127.0.0.1:43123/tasks/1")),
            NavigationDecision::Allow
        );
    }

    #[test]
    fn rejects_urls_that_only_resemble_the_tauri_start_page() {
        assert_eq!(
            decide_navigation(None, &url("http://tauri.localhost.example.com/")),
            NavigationDecision::OpenExternal
        );
        assert_eq!(
            decide_navigation(None, &url("http://tauri.localhost:43123/")),
            NavigationDecision::OpenExternal
        );
    }

    #[test]
    fn opens_external_http_urls_in_system_browser() {
        let origin = url("http://127.0.0.1:43123/");
        assert_eq!(
            decide_navigation(Some(&origin), &url("https://example.com/docs")),
            NavigationDecision::OpenExternal
        );
        assert_eq!(
            decide_navigation(Some(&origin), &url("http://127.0.0.1:43124/")),
            NavigationDecision::OpenExternal
        );
    }

    #[test]
    fn denies_non_http_external_schemes() {
        let origin = url("http://localhost:43123/");
        assert_eq!(
            decide_navigation(Some(&origin), &url("file:///C:/Windows/win.ini")),
            NavigationDecision::Deny
        );
        assert_eq!(
            decide_navigation(Some(&origin), &url("javascript:alert(1)")),
            NavigationDecision::Deny
        );
        assert_eq!(
            decide_navigation(Some(&origin), &url("tauri://localhost/index.html")),
            NavigationDecision::Deny
        );
    }

    #[test]
    fn new_windows_open_only_external_http_urls_in_the_system_browser() {
        let origin = url("http://127.0.0.1:43123/");
        assert_eq!(
            decide_new_window(Some(&origin), &url("https://github.com/example/project")),
            NavigationDecision::OpenExternal
        );
        assert_eq!(
            decide_new_window(Some(&origin), &url("http://127.0.0.1:43123/plugins")),
            NavigationDecision::Deny
        );
        assert_eq!(
            decide_new_window(Some(&origin), &url("file:///C:/Windows/win.ini")),
            NavigationDecision::Deny
        );
        assert_eq!(
            decide_new_window(Some(&origin), &url("javascript:alert(1)")),
            NavigationDecision::Deny
        );
    }
}
