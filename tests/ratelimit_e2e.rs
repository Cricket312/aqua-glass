//! Rate-limit behaviour of the real request path (`services::request_json`) against a
//! local mock HTTP server that answers 429 like Spotify does.

use aqua_glass::services::{ratelimit, request_json, ApiError};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Mock server: the first `n429` requests get 429 with `Retry-After: retry`, the rest 200.
/// Records the arrival time of every request.
fn server(n429: usize, retry: &'static str) -> (u16, Arc<Mutex<Vec<(Instant, u16)>>>) {
    let l = TcpListener::bind("0.0.0.0:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let log = Arc::new(Mutex::new(Vec::new()));
    let log2 = log.clone();
    std::thread::spawn(move || {
        for (i, s) in l.incoming().enumerate() {
            let mut s = match s {
                Ok(s) => s,
                Err(_) => continue,
            };
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut line = String::new();
            // read request head
            loop {
                line.clear();
                if r.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
            }
            let code = if i < n429 { 429 } else { 200 };
            log2.lock().unwrap().push((Instant::now(), code));
            let resp = if code == 429 {
                format!("HTTP/1.1 429 Too Many Requests\r\nRetry-After: {retry}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}")
            } else {
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}".to_string()
            };
            let _ = s.write_all(resp.as_bytes());
        }
    });
    (port, log)
}

fn get(host: &str, port: u16) -> Result<serde_json::Value, ApiError> {
    request_json(reqwest::Method::GET, &format!("http://{host}:{port}/v1/x"), None, &|_| Ok("token".into()))
}

#[test]
fn one_cooldown_for_all_threads_honours_retry_after() {
    aqua_glass::services::install_crypto();
    let (port, log) = server(1, "2");
    let start = Instant::now();
    let handles: Vec<_> = (0..4)
        .map(|_| {
            std::thread::spawn(move || {
                let _bg = ratelimit::background(); // background work waits out the cooldown
                get("127.0.0.1", port)
            })
        })
        .collect();
    for h in handles {
        assert!(h.join().unwrap().is_ok(), "every request eventually succeeds");
    }
    let log = log.lock().unwrap();
    let first429 = log.iter().find(|x| x.1 == 429).unwrap().0;
    // requests already in flight when the 429 arrived may land right after it (they
    // were sent concurrently); nothing *new* may be sent during the cooldown
    let during: Vec<_> = log.iter().filter(|x| x.0 > first429 + Duration::from_millis(150) && x.0 < first429 + Duration::from_millis(1900)).collect();
    assert!(during.is_empty(), "no new request may arrive during the 2 s cooldown: {during:?}");
    let last = log.last().unwrap().0;
    assert!(last >= first429 + Duration::from_secs(2), "the throttled request was retried only after Retry-After");
    assert!(start.elapsed() >= Duration::from_secs(2));
    assert_eq!(log.iter().filter(|x| x.1 == 429).count(), 1, "the server was not hammered");
}

#[test]
fn interactive_request_fails_fast_during_long_cooldown() {
    let (port, log) = server(1, "30");
    let t = Instant::now();
    match get("localhost", port) {
        Err(ApiError::RateLimited(s)) => assert!((25..=31).contains(&s), "reports the remaining wait, got {s}"),
        other => panic!("expected RateLimited, got {other:?}"),
    }
    assert!(t.elapsed() < Duration::from_secs(3), "doesn't hang the UI");
    // a second interactive call is refused locally without contacting the server
    assert!(matches!(get("localhost", port), Err(ApiError::RateLimited(_))));
    assert_eq!(log.lock().unwrap().len(), 1, "only the first request reached the server");
    let st = ratelimit::status("localhost");
    assert!(st.cooldown_left.is_some() && st.total_429 == 1);
}

#[test]
fn bursts_are_paced() {
    let (port, log) = server(0, "0");
    let t = Instant::now();
    for _ in 0..16 {
        assert!(get("127.0.0.2", port).is_ok());
    }
    // default limits: burst 10, then 5 req/s → 6 more requests need >= ~1.2 s
    assert!(t.elapsed() >= Duration::from_millis(1100), "{:?}", t.elapsed());
    assert_eq!(log.lock().unwrap().len(), 16);
}
