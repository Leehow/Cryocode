//! Single-instance plumbing: a Unix socket lets `code <file>` hand paths to the
//! running window in a few milliseconds instead of starting a new process.
//!
//! Protocol: one request per line, `open\t<abs path>[\t<line>[\t<col>]]` or
//! `wait\t<abs path>`; a `wait` connection stays open until that tab closes.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub fn socket_path() -> PathBuf {
    let dir = dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("Cryocode");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("ipc.sock")
}

#[derive(Debug, Clone)]
pub struct OpenRequest {
    pub path: PathBuf,
    pub line: Option<usize>,
    pub col: Option<usize>,
}

impl OpenRequest {
    pub fn encode(&self) -> String {
        let mut s = format!("open\t{}", self.path.display());
        if let Some(l) = self.line {
            s.push_str(&format!("\t{l}"));
            if let Some(c) = self.col {
                s.push_str(&format!("\t{c}"));
            }
        }
        s
    }

    fn decode(fields: &[&str]) -> Option<Self> {
        let path = PathBuf::from(fields.first()?);
        Some(Self { path, line: fields.get(1).and_then(|l| l.parse().ok()), col: fields.get(2).and_then(|c| c.parse().ok()) })
    }
}

/// Inbox shared between the socket thread, the Apple Event handler and the UI.
#[derive(Clone, Default)]
pub struct Inbox {
    pub opens: Arc<Mutex<Vec<OpenRequest>>>,
    pub waiters: Arc<Mutex<Vec<(PathBuf, UnixStream)>>>,
    ctx: Arc<Mutex<Option<egui::Context>>>,
}

impl Inbox {
    pub fn set_ctx(&self, ctx: egui::Context) {
        *self.ctx.lock().unwrap() = Some(ctx);
    }

    pub fn push(&self, req: OpenRequest) {
        self.opens.lock().unwrap().push(req);
        if let Some(ctx) = self.ctx.lock().unwrap().as_ref() {
            ctx.request_repaint();
        }
    }

    pub fn take(&self) -> Vec<OpenRequest> {
        std::mem::take(&mut *self.opens.lock().unwrap())
    }

    /// Release `code --wait` clients whose file is no longer open.
    pub fn release_waiters(&self, still_open: impl Fn(&PathBuf) -> bool) {
        let mut w = self.waiters.lock().unwrap();
        w.retain_mut(|(p, stream)| {
            if still_open(p) {
                true
            } else {
                let _ = stream.write_all(b"done\n");
                false
            }
        });
    }
}

/// Bind the socket, unless another live instance already owns it (then returns None).
pub fn listen(inbox: Inbox) -> Option<()> {
    let path = socket_path();
    if UnixStream::connect(&path).is_ok() {
        return None;
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).ok()?;
    std::thread::Builder::new()
        .name("ipc".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let inbox = inbox.clone();
                std::thread::spawn(move || handle(stream, inbox));
            }
        })
        .ok()?;
    Some(())
}

fn handle(stream: UnixStream, inbox: Inbox) {
    let Ok(read_half) = stream.try_clone() else { return };
    for line in BufReader::new(read_half).lines().map_while(Result::ok) {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields.first().copied() {
            Some("open") => {
                if let Some(r) = OpenRequest::decode(&fields[1..]) {
                    inbox.push(r);
                }
            }
            Some("wait") => {
                if let (Some(p), Ok(s)) = (fields.get(1), stream.try_clone()) {
                    inbox.waiters.lock().unwrap().push((PathBuf::from(p), s));
                }
            }
            Some("ping") => {
                let _ = (&stream).write_all(b"pong\n");
            }
            _ => {}
        }
    }
}

/// Client side: deliver requests to a running instance. Returns the open stream on success.
pub fn send(lines: &[String]) -> Option<UnixStream> {
    let mut s = UnixStream::connect(socket_path()).ok()?;
    for l in lines {
        s.write_all(l.as_bytes()).ok()?;
        s.write_all(b"\n").ok()?;
    }
    s.flush().ok()?;
    Some(s)
}
