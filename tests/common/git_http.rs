#![allow(dead_code)]
//! A git forge on 127.0.0.1, for the push's own tests: real git on both
//! ends, over smart HTTP, and never github.com.
//!
//! The server is `git http-backend`, run as CGI for every request, behind a
//! listener that records what each request carried and answers 401 to any
//! request without the expected token — that 401 is what makes git turn to
//! a credential helper, an askpass or a `.netrc`, so a test can go red when
//! one of those is used.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

/// One request, as it reached the server.
#[derive(Debug, Clone)]
pub struct Seen {
    /// `GET /o/r.git/info/refs?service=git-receive-pack HTTP/1.1`.
    pub line: String,
    /// Every header line as sent.
    pub headers: Vec<String>,
    /// The `Authorization` header, if any.
    pub authorization: Option<String>,
}

pub struct GitServer {
    /// `http://127.0.0.1:<port>/`, with its trailing `/`, the way
    /// [`nunki::forge::github::GIT`] is written.
    pub base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl GitServer {
    /// Serve the bare repositories under `root` (`root/o/r.git`), and let a
    /// request through only with `Basic` credentials `x-access-token` and
    /// `token`.
    pub fn start(root: &Path, token: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let expected = format!(
            "Basic {}",
            base64(format!("{}:{token}", nunki::forge::git::USERNAME).as_bytes())
        );
        let (root, record) = (root.to_path_buf(), seen.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let _ = serve(stream, &root, &expected, &record);
            }
        });
        Self { base, seen }
    }

    /// Every request received so far.
    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

fn serve(
    stream: TcpStream,
    root: &PathBuf,
    expected: &str,
    record: &Arc<Mutex<Vec<Seen>>>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut out = stream;
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let line = line.trim_end().to_string();
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
        headers.push(h.trim_end().to_string());
    }
    let header = |name: &str| {
        headers.iter().find_map(|h| {
            let (k, v) = h.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case(name)
                .then(|| v.trim().to_string())
        })
    };
    if header("expect").is_some_and(|v| v.eq_ignore_ascii_case("100-continue")) {
        out.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
    }
    let body = if header("transfer-encoding").is_some_and(|v| v.contains("chunked")) {
        chunked(&mut reader)?
    } else {
        let length: usize = header("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut body = vec![0; length];
        reader.read_exact(&mut body)?;
        body
    };
    let authorization = header("authorization");
    record.lock().unwrap().push(Seen {
        line: line.clone(),
        headers: headers.clone(),
        authorization: authorization.clone(),
    });

    if authorization.as_deref() != Some(expected) {
        return out.write_all(
            b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"forge\"\r\n\
              Content-Length: 0\r\nConnection: close\r\n\r\n",
        );
    }

    let mut parts = line.split(' ');
    let method = parts.next().unwrap_or("GET").to_string();
    let target = parts.next().unwrap_or("/").to_string();
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let mut cgi = Command::new("git");
    cgi.arg("http-backend")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_PROJECT_ROOT", root)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("REMOTE_USER", nunki::forge::git::USERNAME)
        .env("REMOTE_ADDR", "127.0.0.1")
        .env("REQUEST_METHOD", &method)
        .env("PATH_INFO", path)
        .env("QUERY_STRING", query)
        .env("CONTENT_LENGTH", body.len().to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(v) = header("content-type") {
        cgi.env("CONTENT_TYPE", v);
    }
    if let Some(v) = header("content-encoding") {
        cgi.env("HTTP_CONTENT_ENCODING", v);
    }
    if let Some(v) = header("git-protocol") {
        cgi.env("GIT_PROTOCOL", v);
    }
    let mut child = cgi.spawn()?;
    child.stdin.take().unwrap().write_all(&body)?;
    let produced = child.wait_with_output()?.stdout;

    // CGI: headers, a blank line, the body; `Status:` gives the code.
    let split = produced
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| (i, 4))
        .or_else(|| {
            produced
                .windows(2)
                .position(|w| w == b"\n\n")
                .map(|i| (i, 2))
        });
    let (head, payload) = match split {
        Some((i, n)) => (
            String::from_utf8_lossy(&produced[..i]).to_string(),
            &produced[i + n..],
        ),
        None => (String::new(), &produced[..]),
    };
    let mut status = "200 OK".to_string();
    let mut response = Vec::new();
    for h in head.lines() {
        match h.split_once(':') {
            Some((k, v)) if k.eq_ignore_ascii_case("status") => status = v.trim().to_string(),
            Some(_) => response.push(h.trim_end().to_string()),
            None => {}
        }
    }
    let mut reply = format!("HTTP/1.1 {status}\r\n");
    for h in response {
        reply.push_str(&h);
        reply.push_str("\r\n");
    }
    reply.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    ));
    out.write_all(reply.as_bytes())?;
    out.write_all(payload)
}

fn chunked(reader: &mut impl BufRead) -> std::io::Result<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let mut size = String::new();
        reader.read_line(&mut size)?;
        let size =
            usize::from_str_radix(size.trim().split(';').next().unwrap_or("0"), 16).unwrap_or(0);
        if size == 0 {
            let mut end = String::new();
            reader.read_line(&mut end)?;
            return Ok(body);
        }
        let mut chunk = vec![0; size];
        reader.read_exact(&mut chunk)?;
        body.extend(chunk);
        let mut crlf = [0; 2];
        reader.read_exact(&mut crlf)?;
    }
}

/// Standard base64, padded: what `Basic` credentials are written in.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}
