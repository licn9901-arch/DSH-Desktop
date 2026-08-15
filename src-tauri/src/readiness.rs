//! DSH Host 就绪日志解析。

/// 逐行识别 DSH Host 输出的回环 HTTP 地址。
#[derive(Debug, Default)]
pub struct ReadinessParser {
    ready_url: Option<String>,
}

impl ReadinessParser {
    /// 创建一个尚未观察到就绪地址的解析器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 解析单行日志；第一次识别到合法地址时返回规范化 URL。
    pub fn parse_line(&mut self, line: &str) -> Option<String> {
        if self.ready_url.is_some() {
            return None;
        }

        let start = line.find("http://")?;
        let rest = &line[start..];
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let parsed = url::Url::parse(&rest[..end]).ok()?;
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

        let ready_url = format!("http://{host}:{port}");
        self.ready_url = Some(ready_url.clone());
        Some(ready_url)
    }

    /// 返回是否已经识别到过就绪地址。
    pub fn is_ready(&self) -> bool {
        self.ready_url.is_some()
    }
}
