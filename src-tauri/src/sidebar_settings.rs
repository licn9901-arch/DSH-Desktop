//! Better Sidebar 首次托管安装的安全默认设置初始化。

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::Duration;

use serde_json::{json, Value};
use url::Url;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// 通过 revision-guarded API 关闭 Better Sidebar 的 HTTP/HTTPS 接管。
pub fn initialize_sidebar_defaults(origin: &Url) -> Result<(), String> {
    let current = post_json(origin, "/sidebar/api/settings.get", &json!({}))?;
    let revision = current
        .get("revision")
        .and_then(Value::as_u64)
        .ok_or_else(|| "Better Sidebar settings.get did not return a revision".to_owned())?;
    let payload = json!({
        "patch": {
            "browserInterceptLinks": false,
            "browserInterceptHttp": false,
            "browserInterceptHttps": false
        },
        "expectedRevision": revision
    });
    post_json(origin, "/sidebar/api/settings.update", &payload)?;
    Ok(())
}

/// 只向严格就绪解析器认可的回环 HTTP 原点发送 JSON POST。
fn post_json(origin: &Url, path: &str, payload: &Value) -> Result<Value, String> {
    if origin.scheme() != "http"
        || origin.username() != ""
        || origin.password().is_some()
        || origin.query().is_some()
        || origin.fragment().is_some()
    {
        return Err("sidebar settings origin must be a plain loopback HTTP URL".to_owned());
    }
    let host = origin
        .host_str()
        .ok_or_else(|| "sidebar settings origin has no host".to_owned())?;
    if host != "localhost" && host != "127.0.0.1" {
        return Err("sidebar settings origin is not loopback".to_owned());
    }
    let port = origin
        .port()
        .ok_or_else(|| "sidebar settings origin has no explicit port".to_owned())?;
    let address = resolve_loopback(host, port)?;
    let mut stream = TcpStream::connect_timeout(&address, REQUEST_TIMEOUT)
        .map_err(|error| format!("failed to connect Better Sidebar settings API: {error}"))?;
    stream
        .set_read_timeout(Some(REQUEST_TIMEOUT))
        .map_err(|error| format!("failed to configure settings API timeout: {error}"))?;
    stream
        .set_write_timeout(Some(REQUEST_TIMEOUT))
        .map_err(|error| format!("failed to configure settings API timeout: {error}"))?;

    let body = serde_json::to_vec(payload)
        .map_err(|error| format!("failed to encode sidebar settings request: {error}"))?;
    let request = format!(
        "POST {path} HTTP/1.0\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .and_then(|_| stream.write_all(&body))
        .map_err(|error| format!("failed to write sidebar settings request: {error}"))?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|error| format!("failed to read sidebar settings response: {error}"))?;
    parse_response(&response)
}

/// 将 localhost 固定解析到回环地址，避免使用外部 DNS。
fn resolve_loopback(host: &str, port: u16) -> Result<SocketAddr, String> {
    let ip = match host {
        "127.0.0.1" | "localhost" => IpAddr::from([127, 0, 0, 1]),
        _ => return Err("settings host is not loopback".to_owned()),
    };
    Ok(SocketAddr::new(ip, port))
}

/// 校验 HTTP 状态和插件 wire envelope，仅返回成功的 `value`。
fn parse_response(response: &[u8]) -> Result<Value, String> {
    let separator = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "invalid sidebar settings HTTP response".to_owned())?;
    let headers = std::str::from_utf8(&response[..separator])
        .map_err(|_| "sidebar settings response headers are not UTF-8".to_owned())?;
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| "invalid sidebar settings HTTP status".to_owned())?;
    let envelope: Value = serde_json::from_slice(&response[separator + 4..])
        .map_err(|error| format!("invalid sidebar settings JSON response: {error}"))?;
    if !(200..300).contains(&status) || envelope.get("ok") != Some(&Value::Bool(true)) {
        let message = envelope
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("settings API rejected the request");
        return Err(format!(
            "Better Sidebar settings API failed ({status}): {message}"
        ));
    }
    envelope
        .get("value")
        .cloned()
        .ok_or_else(|| "Better Sidebar settings response has no value".to_owned())
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    use super::initialize_sidebar_defaults;

    /// 读取完整测试请求，避免 Windows 在未消费 body 时复位连接。
    fn read_request(stream: &mut std::net::TcpStream) -> String {
        let mut request = Vec::new();
        loop {
            let mut buffer = [0_u8; 1024];
            let length = stream.read(&mut buffer).unwrap();
            if length == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..length]);
            if let Some(separator) = request.windows(4).position(|value| value == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..separator]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("Content-Length: ")
                            .and_then(|value| value.parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if request.len() >= separator + 4 + content_length {
                    break;
                }
            }
        }
        String::from_utf8(request).unwrap()
    }

    #[test]
    fn settings_are_revision_guarded_and_all_intercepts_are_disabled() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        std::thread::spawn(move || {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                captured.lock().unwrap().push(read_request(&mut stream));
                let body = if index == 0 {
                    r#"{"ok":true,"value":{"value":{},"revision":7}}"#
                } else {
                    r#"{"ok":true,"value":{"value":{},"revision":8}}"#
                };
                write!(
                    stream,
                    "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
        });

        let origin = format!("http://{address}/").parse().unwrap();
        initialize_sidebar_defaults(&origin).unwrap();
        let requests = requests.lock().unwrap();
        assert!(requests[0].starts_with("POST /sidebar/api/settings.get "));
        assert!(requests[1].contains("\"expectedRevision\":7"));
        assert!(requests[1].contains("\"browserInterceptLinks\":false"));
        assert!(requests[1].contains("\"browserInterceptHttp\":false"));
        assert!(requests[1].contains("\"browserInterceptHttps\":false"));
    }

    #[test]
    fn non_loopback_origin_is_rejected_before_network_access() {
        let error =
            initialize_sidebar_defaults(&"http://example.com:8080/".parse().unwrap()).unwrap_err();
        assert!(error.contains("loopback"));
    }
}
