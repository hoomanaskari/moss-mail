//! A stand-in for llama.cpp's `llama-server`, for testing how Moss starts, uses and stops it.
//! It reads the same flags, answers `/health`, and gives canned answers to the decision and
//! chat endpoints, only with the key from `--api-key-file`. The model's file name picks how it
//! acts: `crash` exits at once, `stuck` never gets ready, anything else is ready after a moment.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let model = flag("-m").unwrap_or_default();
    if model.contains("crash") {
        std::process::exit(3);
    }
    let key = flag("--api-key-file").and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
    let port = flag("--port").expect("--port");
    let record = |event: &str| {
        if let Some(path) = flag("--test-startup-log") {
            let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
            file.write_all(format!("{event}\n").as_bytes()).unwrap();
        }
    };
    record("start");
    if let Some(delay) = flag("--test-bind-delay-ms") {
        std::thread::sleep(std::time::Duration::from_millis(delay.parse().unwrap()));
    }
    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).expect("bind");
    record("bound");
    let mut checks = 0;
    for stream in listener.incoming().flatten() {
        let mut reader = BufReader::new(&stream);
        let mut request = String::new();
        if reader.read_line(&mut request).is_err() {
            continue;
        }
        let (mut length, mut auth) = (0, String::new());
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                break;
            }
            let (name, value) = line.split_once(':').unwrap_or_default();
            match name.to_ascii_lowercase().as_str() {
                "content-length" => length = value.trim().parse().unwrap_or(0),
                "authorization" => auth = value.trim().to_string(),
                _ => {}
            }
        }
        let mut body = vec![0; length];
        let _ = reader.read_exact(&mut body);
        let path = request.split_whitespace().nth(1).unwrap_or_default();
        let (status, body) = if path == "/health" {
            if model.contains("slow-health") { std::thread::sleep(std::time::Duration::from_secs(2)); }
            checks += 1;
            if !model.contains("stuck") && checks == 3 { record("ready"); }
            // Still "loading the model" for the first couple of checks.
            if !model.contains("stuck") && checks > 2 {
                ("200 OK", r#"{"status":"ok"}"#)
            } else {
                ("503 Service Unavailable", r#"{"error":"loading"}"#)
            }
        } else if auth != format!("Bearer {key}") {
            ("401 Unauthorized", r#"{"error":"bad key"}"#)
        } else if path == "/v1/systemone" {
            ("200 OK", r#"{"answers":{"reply":{"noul":0.9}}}"#)
        } else {
            // The writing model streams, as Moss always asks it to.
            ("200 OK", "data: {\"choices\":[{\"delta\":{\"content\":\"Reply to the fake.\"}}]}\n\ndata: [DONE]\n\n")
        };
        let _ = write!(
            &stream,
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
    }
}
