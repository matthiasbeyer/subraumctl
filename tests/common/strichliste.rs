//! Local-only fixture for the legacy Strichliste API.
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

pub fn user() -> Value {
    json!({"id": 27, "name": "Me", "balance": 12.345,
    "weightedCountOfPurchases": 10.5, "private": "DO NOT EXPOSE",
    "transactions": [
        {"id": 104, "userId": 27, "createDate": "2026-11-18 18:45:22", "value": 0.01, "comment": "from Other: test"},
        {"id": 103, "userId": 27, "createDate": "2026-11-18 18:44:22", "value": -0.01, "comment": "to Other: test"},
        {"id": 102, "userId": 27, "createDate": "2026-11-18 18:43:22", "value": 10, "comment": "cash"},
        {"id": 101, "userId": 27, "createDate": "2026-11-18 18:42:22", "value": -1.2345, "comment": null}
    ]})
}

pub fn users() -> Value {
    json!({"overallCount": 4, "limit": null, "offset": null, "entries": [
        user(),
        {"id": 30, "name": "Other", "balance": -999, "weightedCountOfPurchases": 15},
        {"id": 31, "name": "Tie", "balance": 0, "weightedCountOfPurchases": 10.5},
        {"id": 32, "name": "Lower", "balance": 100000, "weightedCountOfPurchases": null}
    ]})
}

pub fn transactions() -> Value {
    json!({"overallCount": 4, "limit": 50, "offset": 0, "entries": user()["transactions"]})
}

pub fn transaction_page(offset: usize, limit: usize, total: usize) -> Value {
    let entries: Vec<_> = (offset..offset.saturating_add(limit).min(total))
        .map(|index| {
            json!({"id": total - index, "userId": 27, "createDate": "2026-11-18 18:45:22",
            "value": -0.01, "comment": format!("transaction {}", total - index)})
        })
        .collect();
    json!({"overallCount": total, "limit": limit, "offset": offset, "entries": entries})
}

pub fn server(responses: Vec<(u16, Value)>) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/strich-api", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let task = thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, body) in responses {
            let deadline = Instant::now() + Duration::from_secs(5);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "no HTTP request received");
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 1024];
            loop {
                let n = stream.read(&mut buffer).unwrap();
                assert_ne!(n, 0);
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]);
                    let size = header
                        .lines()
                        .find_map(|line| {
                            line.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|s| s.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + size {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8(bytes).unwrap());
            let body = body.to_string();
            write!(
                stream,
                "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
        requests
    });
    (url, task)
}
