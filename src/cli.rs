//! `code [options] [paths…]` — the terminal entry point.

use crate::ipc::{self, OpenRequest};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const HELP: &str = "Cryocode — 快速的 GPU 原生编辑器

用法: code [选项] [路径…]

  code file.rs          打开文件（已有窗口时在新标签中打开）
  code src/main.rs:42   打开并跳到第 42 行（:行:列 也可以）
  code .                以当前目录作为工作区
  code -w file          等待标签关闭后再返回（适合 git commit 编辑器）
  code -n               只打开窗口

选项:
  -w, --wait            等待文件关闭
  -g, --goto            兼容 VS Code，路径可带 :行:列
  -h, --help            显示帮助
  -v, --version         显示版本
";

pub fn run(args: Vec<String>) -> i32 {
    let mut wait = false;
    let mut paths: Vec<String> = Vec::new();
    for a in args {
        match a.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                return 0;
            }
            "-v" | "--version" => {
                println!("cryocode {}", env!("CARGO_PKG_VERSION"));
                return 0;
            }
            "-w" | "--wait" => wait = true,
            // Accepted for VS Code muscle memory; behaviour is the default here.
            "-g" | "--goto" | "-r" | "--reuse-window" | "-n" | "--new-window" | "-a" | "--add" => {}
            s if s.starts_with('-') && s.len() > 1 => {
                eprintln!("cryocode: 忽略未知选项 {s}");
            }
            _ => paths.push(a),
        }
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    let reqs: Vec<OpenRequest> = paths.iter().map(|p| parse_target(&cwd, p)).collect();
    let mut lines: Vec<String> = reqs.iter().map(OpenRequest::encode).collect();
    if wait {
        lines.extend(reqs.iter().filter(|r| !r.path.is_dir()).map(|r| format!("wait\t{}", r.path.display())));
    }
    if lines.is_empty() {
        lines.push("ping".into());
    }

    if let Some(stream) = ipc::send(&lines) {
        activate_running_app();
        return if wait { block_until_done(stream, reqs.len()) } else { 0 };
    }
    if let Err(e) = launch_gui(&reqs) {
        eprintln!("cryocode: 无法启动窗口：{e}");
        return 1;
    }
    if !wait {
        return 0;
    }
    // The fresh instance received the paths on its command line; register the wait once it listens.
    let deadline = Instant::now() + Duration::from_secs(10);
    let wait_lines: Vec<String> = lines.into_iter().filter(|l| l.starts_with("wait")).collect();
    while Instant::now() < deadline {
        if let Some(stream) = ipc::send(&wait_lines) {
            return block_until_done(stream, wait_lines.len());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    1
}

fn block_until_done(stream: std::os::unix::net::UnixStream, mut remaining: usize) -> i32 {
    let reader = std::io::BufReader::new(stream);
    for line in reader.lines() {
        match line {
            Ok(l) if l == "done" => {
                remaining = remaining.saturating_sub(1);
                if remaining == 0 {
                    return 0;
                }
            }
            Ok(_) => {}
            Err(_) => return 1,
        }
    }
    0
}

/// `path`, `path:line` or `path:line:col`, resolved against `cwd`.
pub fn parse_target(cwd: &Path, raw: &str) -> OpenRequest {
    let abs = |p: &str| -> PathBuf {
        let p = Path::new(p);
        let joined = if p.is_absolute() { p.to_owned() } else { cwd.join(p) };
        normalize(&joined)
    };
    let direct = abs(raw);
    if direct.exists() {
        return OpenRequest { path: direct, line: None, col: None };
    }
    let mut parts = raw.rsplitn(3, ':').collect::<Vec<_>>();
    parts.reverse();
    match parts.as_slice() {
        [p, l, c] if l.parse::<usize>().is_ok() && c.parse::<usize>().is_ok() => {
            OpenRequest { path: abs(p), line: l.parse().ok(), col: c.parse().ok() }
        }
        [p, l] if l.parse::<usize>().is_ok() => OpenRequest { path: abs(p), line: l.parse().ok(), col: None },
        [a, b, l] if l.parse::<usize>().is_ok() => OpenRequest { path: abs(&format!("{a}:{b}")), line: l.parse().ok(), col: None },
        _ => OpenRequest { path: direct, line: None, col: None },
    }
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The `.app` bundle containing this executable, if any.
pub fn bundle_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    exe.ancestors().find(|a| a.extension().is_some_and(|e| e == "app")).map(Path::to_owned)
}

fn launch_gui(reqs: &[OpenRequest]) -> std::io::Result<()> {
    let args: Vec<String> = reqs.iter().map(|r| match (r.line, r.col) {
        (Some(l), Some(c)) => format!("{}:{l}:{c}", r.path.display()),
        (Some(l), None) => format!("{}:{l}", r.path.display()),
        _ => r.path.display().to_string(),
    }).collect();
    if let Some(app) = bundle_path() {
        // Through LaunchServices so the window gets a proper Dock icon and focus.
        Command::new("/usr/bin/open").arg("-a").arg(&app).arg("--args").args(&args).status()?;
        return Ok(());
    }
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe()?;
    Command::new(exe)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()?;
    Ok(())
}

fn activate_running_app() {
    if let Some(app) = bundle_path() {
        let _ = Command::new("/usr/bin/open").arg("-a").arg(app).spawn();
    }
}
