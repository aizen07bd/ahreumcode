use std::io::Read;
use std::time::Duration;

use serde_json::Value;

use super::observation::{ToolErrorKind, ToolObservation};
use crate::tool::{normalize_tool_arguments, ToolName};

pub struct ApprovedWebRequest {
    pub tool_name: String,
    pub arguments: Value,
    pub web_enabled: bool,
    pub timeout_ms: u32,
}

pub fn execute_approved_web(request: ApprovedWebRequest) -> ToolObservation {
    let Some(tool_name) = ToolName::parse(&request.tool_name) else {
        return ToolObservation::failed(
            request.tool_name,
            None,
            ToolErrorKind::UnsupportedTool,
            "web tool is not registered",
        );
    };
    if !matches!(tool_name, ToolName::WebSearch | ToolName::WebFetch) {
        return ToolObservation::failed(
            request.tool_name,
            None,
            ToolErrorKind::UnsupportedTool,
            "tool is not executable by web runtime",
        );
    }
    if !request.web_enabled {
        return ToolObservation::failed(
            tool_name.as_str(),
            web_target_from_arguments(tool_name, &request.arguments),
            ToolErrorKind::PermissionError,
            "web runtime is disabled by configuration",
        );
    }

    let arguments = match normalize_tool_arguments(tool_name, &request.arguments) {
        Ok(arguments) => arguments,
        Err(error) => {
            return ToolObservation::failed(
                tool_name.as_str(),
                web_target_from_arguments(tool_name, &request.arguments),
                ToolErrorKind::InvalidArguments,
                error,
            );
        }
    };

    match tool_name {
        ToolName::WebFetch => execute_web_fetch(&arguments, request.timeout_ms),
        ToolName::WebSearch => execute_web_search(&arguments, request.timeout_ms),
        _ => unreachable!("web tool was checked above"),
    }
}

fn execute_web_fetch(arguments: &Value, timeout_ms: u32) -> ToolObservation {
    let Some(url) = string_arg(arguments, "url") else {
        return ToolObservation::failed(
            ToolName::WebFetch.as_str(),
            None,
            ToolErrorKind::InvalidArguments,
            "missing url",
        );
    };
    let max_bytes = integer_arg(arguments, "max_bytes").unwrap_or(50_000) as usize;
    fetch_url(ToolName::WebFetch.as_str(), url, url, max_bytes, timeout_ms)
}

fn execute_web_search(arguments: &Value, timeout_ms: u32) -> ToolObservation {
    let Some(query) = string_arg(arguments, "query") else {
        return ToolObservation::failed(
            ToolName::WebSearch.as_str(),
            None,
            ToolErrorKind::InvalidArguments,
            "missing query",
        );
    };
    let max_results = integer_arg(arguments, "max_results").unwrap_or(5);
    let url = format!(
        "https://api.duckduckgo.com/?q={}&format=json&no_redirect=1&no_html=1",
        form_encode(query)
    );
    let mut observation = fetch_url(
        ToolName::WebSearch.as_str(),
        query,
        &url,
        200_000,
        timeout_ms,
    );
    if observation.status == super::observation::ObservationStatus::Succeeded {
        observation.preview = web_search_preview(query, max_results, &observation.preview_text());
        observation.total_lines = observation.preview.len();
        observation.total_bytes = observation.preview.join("\n").len();
        observation.message = "web search completed".to_owned();
    }
    observation
}

fn fetch_url(
    tool_name: &str,
    target_raw: &str,
    url: &str,
    max_bytes: usize,
    timeout_ms: u32,
) -> ToolObservation {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_millis(u64::from(timeout_ms)))
        .build();
    let response = match agent.get(url).call() {
        Ok(response) => response,
        Err(error) => return map_web_error(tool_name, target_raw, error),
    };

    let status = response.status();
    let content_type = response
        .header("content-type")
        .map(str::to_owned)
        .unwrap_or_else(|| "-".to_owned());
    let mut bytes = Vec::new();
    let limit = max_bytes.saturating_add(1) as u64;
    if let Err(error) = response.into_reader().take(limit).read_to_end(&mut bytes) {
        return ToolObservation::failed(
            tool_name,
            Some(target_raw.to_owned()),
            ToolErrorKind::IoError,
            format!("web response could not be read: {error}"),
        );
    }

    let source_truncated = bytes.len() > max_bytes;
    if source_truncated {
        bytes.truncate(max_bytes);
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut preview = vec![
        format!("status: {status}"),
        format!("content_type: {content_type}"),
    ];
    preview.extend(
        text.lines()
            .map(str::to_owned)
            .filter(|line| !line.is_empty()),
    );

    ToolObservation::succeeded(
        tool_name,
        Some(target_raw.to_owned()),
        Some(url.to_owned()),
        preview,
        source_truncated,
        None,
        "web request completed",
    )
}

fn web_search_preview(query: &str, max_results: i64, raw: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return vec![
            format!("query: {query}"),
            "result_parse: failed".to_owned(),
            raw.to_owned(),
        ];
    };
    let mut lines = vec![format!("query: {query}")];
    if let Some(abstract_text) = value.get("AbstractText").and_then(Value::as_str) {
        if !abstract_text.is_empty() {
            lines.push(format!("abstract: {abstract_text}"));
        }
    }
    if let Some(abstract_url) = value.get("AbstractURL").and_then(Value::as_str) {
        if !abstract_url.is_empty() {
            lines.push(format!("abstract_url: {abstract_url}"));
        }
    }
    let mut count = 0i64;
    if let Some(topics) = value.get("RelatedTopics").and_then(Value::as_array) {
        for topic in topics {
            if count >= max_results {
                break;
            }
            if let Some(text) = topic.get("Text").and_then(Value::as_str) {
                let url = topic.get("FirstURL").and_then(Value::as_str).unwrap_or("-");
                lines.push(format!("result {}: {text} | {url}", count + 1));
                count += 1;
            }
        }
    }
    if count == 0 && lines.len() == 1 {
        lines.push("results: none".to_owned());
    }
    lines
}

fn map_web_error(tool_name: &str, target_raw: &str, error: ureq::Error) -> ToolObservation {
    match error {
        ureq::Error::Status(status, response) => ToolObservation::failed(
            tool_name,
            Some(target_raw.to_owned()),
            ToolErrorKind::ExecutionError,
            format!(
                "web request returned HTTP {status}: {}",
                response.status_text()
            ),
        ),
        ureq::Error::Transport(transport) => {
            let kind = match transport.kind() {
                ureq::ErrorKind::InvalidUrl | ureq::ErrorKind::UnknownScheme => {
                    ToolErrorKind::InvalidArguments
                }
                ureq::ErrorKind::Io if is_timeout(&transport) => ToolErrorKind::Timeout,
                _ => ToolErrorKind::NetworkError,
            };
            ToolObservation::failed(
                tool_name,
                Some(target_raw.to_owned()),
                kind,
                format!("web request failed: {transport}"),
            )
        }
    }
}

fn is_timeout(transport: &ureq::Transport) -> bool {
    transport
        .to_string()
        .to_ascii_lowercase()
        .contains("timed out")
}

fn web_target_from_arguments(tool_name: ToolName, arguments: &Value) -> Option<String> {
    match tool_name {
        ToolName::WebFetch => arguments
            .get("url")
            .and_then(Value::as_str)
            .map(str::to_owned),
        ToolName::WebSearch => arguments
            .get("query")
            .and_then(Value::as_str)
            .map(str::to_owned),
        _ => None,
    }
}

fn string_arg<'a>(arguments: &'a Value, name: &str) -> Option<&'a str> {
    arguments.get(name).and_then(Value::as_str)
}

fn integer_arg(arguments: &Value, name: &str) -> Option<i64> {
    arguments.get(name).and_then(Value::as_i64)
}

fn form_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(char::from(byte));
            }
            b' ' => encoded.push('+'),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use serde_json::json;

    use super::{execute_approved_web, form_encode, ApprovedWebRequest};

    #[test]
    fn disabled_web_returns_permission_observation() {
        let observation = execute_approved_web(ApprovedWebRequest {
            tool_name: "web_fetch".to_owned(),
            arguments: json!({"url":"https://example.com","max_bytes":1000}),
            web_enabled: false,
            timeout_ms: 1000,
        });

        assert_eq!(observation.status.as_str(), "failed");
        assert_eq!(observation.error_kind.unwrap().as_str(), "permission_error");
    }

    #[test]
    fn web_search_query_encoding_is_not_prompt_specific() {
        assert_eq!(form_encode("rust 1.80 + cargo"), "rust+1.80+%2B+cargo");
    }

    #[test]
    fn fetches_http_url_into_observation() {
        let listener = match TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
            Err(error) => panic!("bind local server: {error}"),
        };
        let url = format!("http://{}", listener.local_addr().expect("addr"));
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buffer = [0u8; 1024];
            let _ = stream.read(&mut buffer);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: 12\r\n\r\nhello web ok",
                )
                .expect("write response");
        });

        let observation = execute_approved_web(ApprovedWebRequest {
            tool_name: "web_fetch".to_owned(),
            arguments: json!({"url":url,"max_bytes":1000}),
            web_enabled: true,
            timeout_ms: 1000,
        });
        handle.join().expect("server thread");

        assert_eq!(observation.status.as_str(), "succeeded");
        assert!(observation.preview_text().contains("hello web ok"));
    }
}
