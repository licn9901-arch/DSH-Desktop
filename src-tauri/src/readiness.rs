//! DSH Host 就绪日志解析。

use std::fmt;

const CORE_READY_PREFIX: &str = "dsh desktop-core: ";
const PLUGINS_READY_PREFIX: &str = "dsh web: ";

/// Host 两级就绪协议解析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadinessSignal {
    /// 核心 Web 服务已可交互，桌面壳可以立即导航。
    CoreReady(String),
    /// 新协议下全部 Loader 插件已经完成。
    PluginsReady(String),
    /// 旧 Host 只输出 `dsh web:`，同时视为核心与插件就绪。
    LegacyReady(String),
}

/// Host 就绪协议中的不可恢复错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadinessError {
    /// 同一 Host 先后报告了两个不同地址，无法确定可信原点。
    ConflictingUrls { first: String, second: String },
}

impl fmt::Display for ReadinessError {
    /// 输出不包含敏感信息之外内容的协议错误说明。
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConflictingUrls { first, second } => {
                write!(
                    formatter,
                    "host reported conflicting readiness URLs: {first} and {second}"
                )
            }
        }
    }
}

/// 逐行识别 DSH Host 输出的唯一回环 HTTP 地址。
#[derive(Debug, Default)]
pub struct ReadinessParser {
    ready_url: Option<String>,
    core_ready: bool,
    plugins_ready: bool,
}

impl ReadinessParser {
    /// 创建一个尚未观察到就绪地址的解析器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 严格解析单行日志，并拒绝冲突的重复地址。
    pub fn parse_line(&mut self, line: &str) -> Result<Option<ReadinessSignal>, ReadinessError> {
        let (candidate, core_signal) = if let Some(candidate) = line.strip_prefix(CORE_READY_PREFIX)
        {
            (candidate, true)
        } else if let Some(candidate) = line.strip_prefix(PLUGINS_READY_PREFIX) {
            (candidate, false)
        } else {
            return Ok(None);
        };
        let Ok(parsed) = url::Url::parse(candidate.trim()) else {
            return Ok(None);
        };
        if parsed.scheme() != "http" {
            return Ok(None);
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Ok(None);
        }
        let Some(host) = parsed.host_str() else {
            return Ok(None);
        };
        if host != "127.0.0.1" && host != "localhost" {
            return Ok(None);
        }
        let Some(port) = parsed.port() else {
            return Ok(None);
        };
        if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
            return Ok(None);
        }

        let ready_url = format!("http://{host}:{port}");
        match &self.ready_url {
            None => {
                self.ready_url = Some(ready_url.clone());
            }
            Some(existing) if existing == &ready_url => {}
            Some(existing) => Err(ReadinessError::ConflictingUrls {
                first: existing.clone(),
                second: ready_url.clone(),
            })?,
        }

        if core_signal {
            if self.core_ready {
                return Ok(None);
            }
            self.core_ready = true;
            return Ok(Some(ReadinessSignal::CoreReady(ready_url)));
        }

        if self.plugins_ready {
            return Ok(None);
        }
        self.plugins_ready = true;
        let legacy = !self.core_ready;
        self.core_ready = true;
        Ok(Some(if legacy {
            ReadinessSignal::LegacyReady(ready_url)
        } else {
            ReadinessSignal::PluginsReady(ready_url)
        }))
    }

    /// 返回是否已经识别到核心就绪地址。
    pub fn is_core_ready(&self) -> bool {
        self.core_ready
    }

    /// 返回是否已经识别到全部插件就绪信号。
    pub fn is_plugins_ready(&self) -> bool {
        self.plugins_ready
    }
}

#[cfg(test)]
mod tests {
    use super::{ReadinessError, ReadinessParser, ReadinessSignal};

    #[test]
    fn parses_core_then_plugins_readiness_once() {
        let mut parser = ReadinessParser::new();
        assert_eq!(
            parser
                .parse_line("dsh desktop-core: http://127.0.0.1:4321")
                .unwrap(),
            Some(ReadinessSignal::CoreReady(
                "http://127.0.0.1:4321".to_owned()
            ))
        );
        assert_eq!(
            parser
                .parse_line("dsh desktop-core: http://127.0.0.1:4321")
                .unwrap(),
            None
        );
        assert_eq!(
            parser.parse_line("dsh web: http://127.0.0.1:4321").unwrap(),
            Some(ReadinessSignal::PluginsReady(
                "http://127.0.0.1:4321".to_owned()
            ))
        );
        assert!(parser.is_core_ready());
        assert!(parser.is_plugins_ready());
    }

    #[test]
    fn legacy_plugins_signal_also_marks_core_ready() {
        let mut parser = ReadinessParser::new();
        assert_eq!(
            parser
                .parse_line("dsh web: http://localhost:4321/")
                .unwrap(),
            Some(ReadinessSignal::LegacyReady(
                "http://localhost:4321".to_owned()
            ))
        );
        assert!(parser.is_core_ready());
        assert!(parser.is_plugins_ready());
    }

    #[test]
    fn accepts_only_exact_prefixed_loopback_urls() {
        let mut parser = ReadinessParser::new();
        assert_eq!(
            parser.parse_line("dsh web: http://127.0.0.1:4321").unwrap(),
            Some(ReadinessSignal::LegacyReady(
                "http://127.0.0.1:4321".to_owned()
            ))
        );
        assert!(parser.is_core_ready());
    }

    #[test]
    fn rejects_url_without_exact_dsh_prefix() {
        let mut parser = ReadinessParser::new();
        assert_eq!(
            parser
                .parse_line("attacker says http://127.0.0.1:4321")
                .unwrap(),
            None
        );
    }

    #[test]
    fn rejects_paths_queries_fragments_and_non_loopback_hosts() {
        let invalid_lines = [
            "dsh web: http://127.0.0.1:4321/admin",
            "dsh web: http://localhost:4321/?token=secret",
            "dsh web: http://localhost:4321/#fragment",
            "dsh web: http://0.0.0.0:4321/",
            "dsh web: https://127.0.0.1:4321/",
            "dsh web: http://127.0.0.1/",
            "dsh web: http://user:password@127.0.0.1:4321/",
            "prefix dsh web: http://127.0.0.1:4321/",
        ];

        for line in invalid_lines {
            assert_eq!(
                ReadinessParser::new().parse_line(line).unwrap(),
                None,
                "{line}"
            );
        }
    }

    #[test]
    fn permits_same_duplicate_and_rejects_conflicting_duplicate() {
        let mut parser = ReadinessParser::new();
        parser
            .parse_line("dsh desktop-core: http://localhost:4321/")
            .unwrap();
        assert_eq!(
            parser
                .parse_line("dsh desktop-core: http://localhost:4321/")
                .unwrap(),
            None
        );
        assert_eq!(
            parser.parse_line("dsh web: http://localhost:4322/"),
            Err(ReadinessError::ConflictingUrls {
                first: "http://localhost:4321".to_owned(),
                second: "http://localhost:4322".to_owned(),
            })
        );
    }
}
