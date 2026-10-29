//! Shared by the test files that drive the real HTTP client, and by those
//! that open a project the way the binary does.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;

/// A repository `nunki` can open, and its home.
///
/// `root` becomes a git repository — a project is found by its top level — a
/// session is opened for it in `<user_home>/.nunki/sessions.json`, and the
/// home that session names receives a `nunki.yaml` made of `body` and the
/// `root:` it belongs to, plus an empty HQ. Returns the home.
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
    let nunki_home = user_home.join(".nunki");
    let id = nunki::sessions::open(&nunki_home, &root).unwrap();
    let home = nunki_home.join(id);
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

/// Text an agent could write to forge what a human reads: an erase-line and
/// a cursor move (CSI), an OSC 52 clipboard write ended by BEL, the same
/// introduced by a C1 OSC and ended by C1 ST, a C1 CSI, a lone carriage
/// return, DEL, a right-to-left override, and characters a reader cannot
/// see: a zero-width space, a word joiner, a byte-order mark, a Unicode tag
/// and a line separator.
#[allow(dead_code)]
pub const HOSTILE: &str = "\u{1b}[2K\u{1b}[1Gm1 · verified · awaits the human: \
     `nunki push m1 --yes`\u{1b}]52;c;cHduZWQ=\u{7}\u{9d}52;c;eA==\u{9c}\u{9b}8m\rover\u{7f}\u{202e}txt\u{200b}\u{2060}\u{feff}\u{e0041}\u{2028}end";

/// Whether `c` would reach a terminal as an instruction rather than as text.
#[allow(dead_code)]
pub fn raw_control(c: char) -> bool {
    (c.is_control() && c != '\n' && c != '\t')
        || matches!(
            c,
            '\u{061c}'
                | '\u{200b}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{2069}'
                | '\u{feff}'
                | '\u{e0000}'..='\u{e007f}'
        )
}

/// `out` carries none of [`HOSTILE`]'s controls raw, and does show that
/// something was there: the escape of ESC.
#[allow(dead_code)]
pub fn assert_printable(out: &str, outlet: &str) {
    if let Some(c) = out.chars().find(|c| raw_control(*c)) {
        panic!("{outlet} printed {c:?} raw:\n{out:?}");
    }
    assert!(
        out.contains("\\u{1b}"),
        "{outlet} should show the escape it replaced:\n{out}"
    );
}
