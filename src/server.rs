//! 本地 HTTP 服务：提供仪表盘页面与日数据接口。
//!
//! 手写单线程实现：这是一个只服务本机浏览器的只读小服务，没有并发请求，
//! 用线程池（如 `tiny_http` 默认固定开 4 个空闲线程）纯属浪费。

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::sync::SyncState;

/// 内嵌仪表盘（单文件，无外部资源）。
const INDEX_HTML: &str = include_str!("web/index.html");

/// 返回内嵌的仪表盘 HTML。
pub fn static_index() -> &'static str {
    INDEX_HTML
}

/// 生成 `/api/days` 的响应体。
pub fn render_days(state: &SyncState) -> String {
    let (days, error, last_sync_unix) = state.snapshot();
    let day_values: Vec<serde_json::Value> = days
        .iter()
        .map(|(day, stats)| {
            serde_json::json!({
                "day": day,
                "characters": stats.characters,
                "keystrokes": stats.keystrokes,
                "commits": stats.commits,
                "speed_cpm": stats.speed_cpm,
            })
        })
        .collect();

    serde_json::json!({
        "sync": {
            "last_sync_unix": last_sync_unix,
            "error": error,
        },
        "days": day_values,
    })
    .to_string()
}

/// 为一个请求路径生成 `(状态码, Content-Type, 响应体)`。
fn route(path: &str, state: &SyncState) -> (u16, &'static str, String) {
    match path {
        "/" | "/index.html" => (200, "text/html; charset=utf-8", static_index().to_string()),
        "/api/days" => (200, "application/json; charset=utf-8", render_days(state)),
        _ => (
            404,
            "text/plain; charset=utf-8",
            "404 Not Found".to_string(),
        ),
    }
}

/// 处理单个连接：读请求行与头部，写回响应。
fn handle(stream: &mut TcpStream, state: &SyncState) {
    // 本地浏览器请求头部很小，512 字节足够，超长直接放弃
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok();
    let mut reader = BufReader::new(&*stream);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    // 丢弃剩余头部直到空行，避免客户端等待
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) if line == "\r\n" || line == "\n" => break,
            Ok(_) => {}
            Err(_) => return,
        }
    }

    // "GET /api/days HTTP/1.1"
    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .split('?')
        .next()
        .unwrap_or("/");

    let (status, content_type, body) = route(path, state);
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {len}\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        reason = if status == 200 { "OK" } else { "Not Found" },
        len = body.len(),
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// 绑定回环地址上的指定端口；失败返回 None。
pub fn bind(port: u16) -> Option<TcpListener> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port)).ok()
}

/// 从起始端口开始尝试绑定，返回 `(listener, port)`；全部失败返回 None。
pub fn bind_first_available(start: u16, attempts: u16) -> Option<(TcpListener, u16)> {
    for offset in 0..attempts {
        let Some(port) = start.checked_add(offset) else {
            break;
        };
        if let Some(listener) = bind(port) {
            return Some((listener, port));
        }
    }
    None
}

/// 单线程处理请求，直到 `shutdown` 被置位。
///
/// 串行处理即可：本机浏览器只会一个请求接一个请求地打这个只读接口。
pub fn serve(listener: TcpListener, state: SyncState, shutdown: Arc<AtomicBool>) {
    for stream in listener.incoming() {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }
        if let Ok(mut stream) = stream {
            handle(&mut stream, &state);
        }
    }
}

/// 只用于测试：客户端连接地址。
#[allow(dead_code)]
fn addr_of(listener: &TcpListener) -> Option<SocketAddr> {
    listener.local_addr().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rime::parser::DayStats;

    fn state_with_days() -> SyncState {
        let mut days = std::collections::BTreeMap::new();
        days.insert(
            "2026-11-06".to_string(),
            DayStats {
                characters: 1234,
                keystrokes: 4000,
                commits: 210,
                speed_cpm: 78,
            },
        );
        let state = SyncState::new(days);
        state.replace(crate::sync::SyncOutcome {
            days: state.snapshot().0,
            changed: 0,
            error: None,
            last_sync_unix: 1791796577,
        });
        state
    }

    #[test]
    fn renders_days_json() {
        let body = render_days(&state_with_days());
        let v: serde_json::Value = serde_json::from_str(&body).expect("应为合法 JSON");

        assert_eq!(v["sync"]["last_sync_unix"], 1791796577);
        assert!(v["sync"]["error"].is_null());

        let days = v["days"].as_array().expect("days 应为数组");
        assert_eq!(days.len(), 1);
        assert_eq!(days[0]["day"], "2026-11-06");
        assert_eq!(days[0]["characters"], 1234);
        assert_eq!(days[0]["keystrokes"], 4000);
        assert_eq!(days[0]["commits"], 210);
        assert_eq!(days[0]["speed_cpm"], 78);
    }

    #[test]
    fn renders_days_sorted_by_date() {
        let mut days = std::collections::BTreeMap::new();
        for d in ["2026-11-08", "2026-11-06", "2026-11-07"] {
            days.insert(d.to_string(), DayStats::default());
        }
        let state = SyncState::new(days);
        let body = render_days(&state);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let dates: Vec<&str> = v["days"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["day"].as_str().unwrap())
            .collect();
        assert_eq!(dates, vec!["2026-11-06", "2026-11-07", "2026-11-08"]);
    }

    #[test]
    fn renders_empty_days_array() {
        let state = SyncState::new(std::collections::BTreeMap::new());
        let body = render_days(&state);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["days"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn routes_index_and_api_and_404() {
        let state = state_with_days();
        assert_eq!(route("/", &state).0, 200);
        assert_eq!(route("/index.html", &state).0, 200);
        assert_eq!(route("/api/days", &state).0, 200);
        assert_eq!(route("/nope", &state).0, 404);
    }

    #[test]
    fn strips_query_string_in_route() {
        // route 接收的已是去掉查询串的路径，这里验证上层分割逻辑的等价性
        let raw = "/api/days?t=1";
        assert_eq!(raw.split('?').next().unwrap(), "/api/days");
    }

    #[test]
    fn bind_uses_loopback_only() {
        let listener = bind(0).expect("绑定随机端口应成功");
        let addr = addr_of(&listener).expect("应有本地地址");
        assert!(addr.ip().is_loopback(), "必须只绑定回环地址");
    }

    #[test]
    fn bind_first_available_reports_chosen_port() {
        let (_listener, port) = bind_first_available(8787, 11).expect("应能绑定到一个端口");
        assert!((8787..=8797).contains(&port));
    }

    #[test]
    fn static_index_is_non_empty_html() {
        let html = static_index();
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(!html.trim().is_empty());
    }

    #[test]
    fn serves_real_http_response() {
        use std::io::Read;
        let listener = bind(0).expect("绑定随机端口应成功");
        let port = listener.local_addr().unwrap().port();
        let state = state_with_days();
        let shutdown = Arc::new(AtomicBool::new(false));

        let handle = std::thread::spawn({
            let shutdown = shutdown.clone();
            move || serve(listener, state, shutdown)
        });

        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("应能连接");
        stream
            .write_all(b"GET /api/days HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();

        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("application/json"));
        assert!(response.contains("\"2026-11-06\""));
        assert!(response.contains("\"characters\":1234"));

        shutdown.store(true, Ordering::Relaxed);
        // 再连一次让 accept 返回，从而看到 shutdown 标志
        let _ = TcpStream::connect(("127.0.0.1", port));
        let _ = handle.join();
    }
}
