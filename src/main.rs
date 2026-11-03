//! Cryocode — a fast, GPU-native editor for code, Markdown, LaTeX and data files.

mod app;
mod chrome;
mod cli;
mod document;
mod editor;
mod formats;
mod highlight;
mod ipc;
#[cfg(target_os = "macos")]
mod macos;
mod math;
mod palette;
mod preview;
mod search;
mod sidebar;
mod syntax_theme;
mod terminal;
mod theme;

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--cli") {
        std::process::exit(cli::run(args[1..].to_vec()));
    }

    let cwd = std::env::current_dir().unwrap_or_default();
    let requests: Vec<ipc::OpenRequest> = args
        .iter()
        .filter(|a| !a.starts_with('-'))
        .map(|a| cli::parse_target(&cwd, a))
        .collect();

    // Single instance: if a window is already running, hand it the files and leave.
    let inbox = ipc::Inbox::default();
    if ipc::listen(inbox.clone()).is_none() {
        let mut lines: Vec<String> = requests.iter().map(ipc::OpenRequest::encode).collect();
        lines.push("ping".into());
        let _ = ipc::send(&lines);
        return;
    }
    #[cfg(target_os = "macos")]
    macos::install(inbox.clone());

    let viewport = egui::ViewportBuilder::default()
        .with_title("Cryocode")
        .with_app_id("dev.cryocode.Cryocode")
        .with_inner_size([1280.0, 820.0])
        .with_min_inner_size([560.0, 360.0])
        .with_fullsize_content_view(true)
        .with_titlebar_shown(false)
        .with_title_shown(false)
        .with_drag_and_drop(true);
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        vsync: true,
        persist_window: true,
        ..Default::default()
    };
    let result = eframe::run_native("Cryocode", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, inbox, requests)))));
    let _ = std::fs::remove_file(ipc::socket_path());
    if let Err(e) = result {
        eprintln!("cryocode: {e}");
        std::process::exit(1);
    }
}

/// Install the `code` shell command into ~/.local/bin, pointing at this executable.
pub fn install_cli() -> std::io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let exe = std::env::current_exe()?.canonicalize()?;
    let dir = dirs::home_dir().ok_or_else(|| std::io::Error::other("no home"))?.join(".local/bin");
    std::fs::create_dir_all(&dir)?;
    let target = dir.join("code");
    let script = format!("#!/bin/sh\n# Installed by Cryocode\nexec \"{}\" --cli \"$@\"\n", exe.display());
    std::fs::write(&target, script)?;
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))?;
    Ok(target)
}

