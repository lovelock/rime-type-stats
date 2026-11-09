//! 本地 HTTP 服务：提供仪表盘页面与日数据接口。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tiny_http::{Header, Response, Server};

use crate::sync::SyncState;

/// 内嵌仪表盘（单文件，无外部资源）。
const INDEX_HTML: &str = include_str!("web/index.html");

/// 返回内嵌的仪表盘 HTML。
pub fn static_index() -> &'static str {
    INDEX_HTML
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("响应头字面量必须合法")
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

/// 绑定回环地址上的指定端口；失败返回 None。
pub fn bind(port: u16) -> Option<Server> {
    Server::http(("127.0.0.1", port)).ok()
}

/// 从起始端口开始尝试绑定，返回 `(server, port)`；全部失败返回 None。
pub fn bind_first_available(start: u16, attempts: u16) -> Option<(Server, u16)> {
    for offset in 0..attempts {
        let Some(port) = start.checked_add(offset) else {
            break;
        };
        if let Some(server) = bind(port) {
            return Some((server, port));
        }
    }
    None
}

/// 处理请求直到 `shutdown` 被置位。
pub fn serve(server: Server, state: SyncState, shutdown: Arc<AtomicBool>) {
    for request in server.incoming_requests() {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let url = request.url().to_string();
        let path = url.split('?').next().unwrap_or("");

        let response = match path {
            "/" | "/index.html" => Response::from_string(static_index())
                .with_header(header("Content-Type", "text/html; charset=utf-8"))
                .with_header(header("Cache-Control", "no-store")),
            "/api/days" => Response::from_string(render_days(&state))
                .with_header(header("Content-Type", "application/json; charset=utf-8"))
                .with_header(header("Cache-Control", "no-store")),
            _ => Response::from_string("404 Not Found")
                .with_status_code(404)
                .with_header(header("Content-Type", "text/plain; charset=utf-8")),
        };

        let _ = request.respond(response);
    }
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
    fn bind_uses_loopback_only() {
        let server = bind(0).expect("绑定随机端口应成功");
        let addr = server.server_addr();
        assert!(
            addr.to_ip().expect("应绑定到 IP 地址").ip().is_loopback(),
            "必须只绑定回环地址"
        );
    }

    #[test]
    fn bind_first_available_reports_chosen_port() {
        let (_server, port) = bind_first_available(8787, 11).expect("应能绑定到一个端口");
        assert!((8787..=8797).contains(&port));
    }

    #[test]
    fn static_index_is_non_empty_html() {
        let html = static_index();
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(!html.trim().is_empty());
    }
}
