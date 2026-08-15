//! DSH Host 就绪日志解析。

use std::fmt;

const READY_PREFIX: &str = "dsh web: ";

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
}

impl ReadinessParser {
    /// 创建一个尚未观察到就绪地址的解析器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 严格解析单行日志，并拒绝冲突的重复地址。
    pub fn parse_line(&mut self, line: &str) -> Result<Option<String>, ReadinessError> {
        let Some(candidate) = line.strip_prefix(READY_PREFIX) else {
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
                Ok(Some(ready_url))
            }
            Some(existing) if existing == &ready_url => Ok(None),
            Some(existing) => Err(ReadinessError::ConflictingUrls {
                first: existing.clone(),
                second: ready_url,
            }),
        }
    }

    /// 返回是否已经识别到过就绪地址。
    pub fn is_ready(&self) -> bool {
        self.ready_url.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::{ReadinessError, ReadinessParser};

    #[test]
    fn accepts_only_exact_prefixed_loopback_urls() {
        let mut parser = ReadinessParser::new();
        assert_eq!(
            parser.parse_line("dsh web: http://127.0.0.1:4321").unwrap(),
            Some("http://127.0.0.1:4321".to_owned())
        );
        assert!(parser.is_ready());
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
            .parse_line("dsh web: http://localhost:4321/")
            .unwrap();
        assert_eq!(
            parser
                .parse_line("dsh web: http://localhost:4321/")
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
