//! Shared by the test files that drive the real HTTP client, and by those
//! that open a project the way the binary does.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;

/// A repository `nunki` can open, and its home.
///
/// `root` becomes a git repository — a project is found by its top level —
/// and the home `nunki` derives from it, `<user_home>/.nunki/<name>/`,
/// receives a `nunki.yaml` made of `body` and the `root:` it belongs to, and
/// an empty HQ. Returns the home.
#[allow(dead_code)]
pub fn project_home(root: &Path, user_home: &Path, body: &str) -> PathBuf {
    std::fs::create_dir_all(root).unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .arg(root)
            .status()
            .unwrap()
            .success()
    );
    let root = std::fs::canonicalize(root).unwrap();
    let home = user_home.join(".nunki").join(root.file_name().unwrap());
    std::fs::create_dir_all(home.join(nunki::project::HQ_DIR)).unwrap();
    std::fs::write(
        home.join(nunki::project::CONFIG_FILE),
        format!("root: {}\n{body}", root.display()),
    )
    .unwrap();
    home
}

/// A server that answers each connection with the next canned response, and
/// hands back what it was sent, as a server receives it. `Connection: close`
/// on every answer, so each request is its own connection and the order is
/// the order asked.
///
/// A fake forge would prove the fake. What a forge client gets wrong silently
/// is the request's shape — a missing `Authorization` header gets a 404 from
/// GitHub, which reads as a repository that does not exist — so the request
/// is read byte by byte.
#[allow(dead_code)]
pub fn serve(responses: Vec<(u16, &'static str)>) -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for (status, body) in responses {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let (mut request, mut length) = (String::new(), 0usize);
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
                request.push_str(&line);
                if line == "\r\n" || line.is_empty() {
                    break;
                }
            }
            let mut payload = vec![0; length];
            reader.read_exact(&mut payload).unwrap();
            request.push_str(&String::from_utf8_lossy(&payload));
            seen.push(request);
            let mut stream = stream;
            write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
        seen
    });
    (base, handle)
}
