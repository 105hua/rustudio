//! A small local HTTP server standing in for Roblox's hosts, so saving and
//! publishing can be tested without writing anything to Roblox (unit tests
//! and the self-test). [`crate::cloud::Endpoints::mock`] points every host
//! at it as `http://127.0.0.1:PORT/<host>/…`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// One request the server received.
#[derive(Debug, Clone)]
pub struct MockRequest {
    pub method: String,
    /// Path and query, e.g. `/assetdelivery/v2/assetId/1`.
    pub path: String,
    /// Header names lower-cased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl MockRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// A reply: status, headers, body.
pub struct MockReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl MockReply {
    pub fn json(status: u16, body: &str) -> MockReply {
        MockReply {
            status,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: body.as_bytes().to_vec(),
        }
    }

    pub fn with_header(mut self, k: &str, v: &str) -> MockReply {
        self.headers.push((k.into(), v.into()));
        self
    }
}

pub type Handler = Arc<dyn Fn(&MockRequest) -> MockReply + Send + Sync>;

pub struct MockServer {
    pub base: String,
    requests: Arc<Mutex<Vec<MockRequest>>>,
    stop: Arc<AtomicBool>,
    port: u16,
}

impl MockServer {
    /// Listens on a free 127.0.0.1 port; `handler` answers every request.
    pub fn start(handler: Handler) -> std::io::Result<MockServer> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (reqs, stop2) = (requests.clone(), stop.clone());
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                if stop2.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(conn) = conn else { continue };
                let (reqs, handler) = (reqs.clone(), handler.clone());
                std::thread::spawn(move || {
                    let _ = serve(conn, &reqs, &handler);
                });
            }
        });
        Ok(MockServer {
            base: format!("http://127.0.0.1:{port}"),
            requests,
            stop,
            port,
        })
    }

    /// Every request received so far, in order.
    pub fn requests(&self) -> Vec<MockRequest> {
        self.requests.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wakes the accept loop so it sees the flag.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

fn serve(conn: TcpStream, reqs: &Mutex<Vec<MockRequest>>, handler: &Handler) -> std::io::Result<()> {
    let mut reader = BufReader::new(conn.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    if method.is_empty() {
        return Ok(());
    }
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    let len: usize = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; len];
    reader.read_exact(&mut body)?;
    let req = MockRequest {
        method,
        path,
        headers,
        body,
    };
    let reply = handler(&req);
    if let Ok(mut g) = reqs.lock() {
        g.push(req);
    }
    let mut out = conn;
    let mut head = format!("HTTP/1.1 {} Mock\r\nContent-Length: {}\r\nConnection: close\r\n", reply.status, reply.body.len());
    for (k, v) in &reply.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    out.write_all(head.as_bytes())?;
    out.write_all(&reply.body)?;
    out.flush()
}
