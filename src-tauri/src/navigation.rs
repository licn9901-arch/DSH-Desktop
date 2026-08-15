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

/// 根据当前 Host 原点判断目标地址如何处理。
pub fn decide_navigation(host_origin: Option<&url::Url>, target: &url::Url) -> NavigationDecision {
    if host_origin.is_none() && target.scheme() == "tauri" && target.host_str() == Some("localhost")
    {
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

#[cfg(test)]
mod tests {
    use super::{decide_navigation, NavigationDecision};

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
            decide_navigation(Some(&origin), &url("http://127.0.0.1:43123/tasks/1")),
            NavigationDecision::Allow
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
}
