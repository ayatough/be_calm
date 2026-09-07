#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod fonts;
mod platform;

use std::fs::File;

fn init_logging() {
    let dir = be_calm_core::config::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let cfg = simplelog::ConfigBuilder::new()
        .set_time_format_rfc3339()
        .build();
    let mut loggers: Vec<Box<dyn simplelog::SharedLogger>> = Vec::new();
    if let Ok(f) = File::create(dir.join("be_calm.log")) {
        loggers.push(simplelog::WriteLogger::new(
            log::LevelFilter::Info,
            cfg.clone(),
            f,
        ));
    }
    #[cfg(debug_assertions)]
    loggers.push(simplelog::TermLogger::new(
        log::LevelFilter::Debug,
        cfg,
        simplelog::TerminalMode::Mixed,
        simplelog::ColorChoice::Auto,
    ));
    let _ = simplelog::CombinedLogger::init(loggers);
}

/// Run the process watcher with the saved config for `secs` seconds and print
/// what it blocks. Does not touch the shell.
fn dry_watch(secs: u64) {
    use be_calm_core::{config::config_path, Config, Policy};
    let cfg = Config::load(&config_path()).unwrap_or_default();
    let policy = Policy::new(
        &cfg.allowed_apps,
        &platform::process::system_roots(),
        std::env::current_exe().ok().as_deref(),
    );
    println!(
        "allowed: {:?}",
        cfg.allowed_apps
            .iter()
            .map(|a| a.path.display().to_string())
            .collect::<Vec<_>>()
    );
    let watcher = platform::watcher::Watcher::start(
        policy,
        platform::watcher::WatchOptions {
            block_windowless: cfg.block_windowless,
            guard_foreground: cfg.guard_foreground,
        },
    );
    let end = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    while std::time::Instant::now() < end {
        while let Some(ev) = watcher.try_recv() {
            println!("{ev:?}");
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    drop(watcher);
    println!("dry-watch finished");
}

fn main() -> eframe::Result<()> {
    init_logging();
    let args: Vec<String> = std::env::args().collect();

    // `be_calm --restore`: put the shell back and exit. Useful if something
    // went badly wrong and the taskbar is still hidden.
    if args.iter().any(|a| a == "--restore") {
        platform::shell::restore_all();
        platform::shell::clear_recovery_marker();
        return Ok(());
    }

    // Diagnostics without the GUI. See docs/spec.md "CLI".
    if args.iter().any(|a| a == "--list-windows") {
        for a in platform::process::windowed_apps() {
            println!("{:>6}  {}  {}", a.pid, a.exe.display(), a.title);
        }
        return Ok(());
    }
    if args.iter().any(|a| a == "--shell-test") {
        // Hide the taskbar and desktop icons for a few seconds, then restore.
        platform::shell::write_recovery_marker(None);
        platform::shell::hide_taskbar();
        platform::shell::hide_desktop_icons();
        std::thread::sleep(std::time::Duration::from_secs(6));
        platform::shell::restore_all();
        platform::shell::clear_recovery_marker();
        println!("shell-test finished");
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--dry-watch") {
        let secs: u64 = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(10);
        dry_watch(secs);
        return Ok(());
    }

    // Whatever happens, never leave the desktop without a taskbar.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        platform::shell::restore_all();
        platform::shell::clear_recovery_marker();
        default_hook(info);
    }));

    // Crash recovery: if a previous run left the shell hidden, fix it first.
    if platform::shell::recovery_marker_exists() {
        log::warn!("previous session did not end cleanly; restoring shell");
        platform::shell::restore_all();
        platform::shell::clear_recovery_marker();
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("be_calm")
            .with_inner_size([520.0, 560.0])
            .with_min_inner_size([420.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "be_calm",
        options,
        Box::new(|cc| {
            fonts::install(&cc.egui_ctx);
            // A little larger than egui's default; Ctrl+= / Ctrl+- still work.
            cc.egui_ctx.set_zoom_factor(1.2);
            Ok(Box::new(app::BeCalmApp::new()))
        }),
    )
}
