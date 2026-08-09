use std::{
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, RwLock},
    thread,
    time::Duration,
};

use crate::{metrics::render_prometheus, procfs::Snapshot};

pub fn serve(address: &str, state: Arc<RwLock<Snapshot>>, process_limit: usize) -> io::Result<()> {
    let listener = TcpListener::bind(address)?;
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = Arc::clone(&state);
                thread::spawn(move || handle(stream, state, process_limit));
            }
            Err(error) => eprintln!("bbtop exporter connection: {error}"),
        }
    }
    Ok(())
}

fn handle(mut stream: TcpStream, state: Arc<RwLock<Snapshot>>, process_limit: usize) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut request = [0_u8; 2048];
    let length = match stream.read(&mut request) {
        Ok(length) => length,
        Err(_) => return,
    };
    let first_line = String::from_utf8_lossy(&request[..length]);
    let path = first_line.split_whitespace().nth(1).unwrap_or("/");
    let (status, content_type, body) = match path {
        "/metrics" => (
            "200 OK",
            "text/plain; version=0.0.4; charset=utf-8",
            render_prometheus(&state.read().unwrap(), process_limit),
        ),
        "/healthz" => ("200 OK", "text/plain; charset=utf-8", "ok\n".into()),
        "/" => (
            "200 OK",
            "text/plain; charset=utf-8",
            "bbtop exporter\n\nGET /metrics\nGET /healthz\n".into(),
        ),
        _ => (
            "404 Not Found",
            "text/plain; charset=utf-8",
            "not found\n".into(),
        ),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}
