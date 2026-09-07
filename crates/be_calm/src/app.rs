//! The egui application: setup screen -> focus screen -> summary.

use crate::platform::watcher::{WatchEvent, WatchOptions, Watcher};
use crate::platform::{process, shell, WindowedApp};
use be_calm_core::config::{config_path, file_stem_of, MAX_ALLOWED_APPS};
use be_calm_core::session::{exit_challenge_passed, format_clock};
use be_calm_core::{AllowedApp, Config, Policy, Session, SessionSummary};
use egui::{Color32, RichText, ViewportCommand, WindowLevel};
use std::time::{Duration, Instant};

const TOAST_TTL: Duration = Duration::from_secs(4);

enum Screen {
    Setup,
    Focus(FocusState),
    Done(SessionSummary),
}

struct FocusState {
    session: Session,
    watcher: Watcher,
    show_exit: bool,
    exit_input: String,
    toast: Option<(String, Instant)>,
}

pub struct BeCalmApp {
    cfg: Config,
    screen: Screen,
    picker: Option<Vec<WindowedApp>>,
    error: Option<String>,
}

impl BeCalmApp {
    pub fn new() -> Self {
        let cfg = match Config::load(&config_path()) {
            Ok(c) => c,
            Err(e) => {
                log::error!("config load failed: {e}");
                Config::default()
            }
        };
        Self {
            cfg,
            screen: Screen::Setup,
            picker: None,
            error: None,
        }
    }

    fn save_config(&mut self) {
        if let Err(e) = self.cfg.save(&config_path()) {
            log::error!("config save failed: {e}");
            self.error = Some(format!("設定の保存に失敗: {e}"));
        }
    }

    fn add_app(&mut self, app: AllowedApp) {
        match self.cfg.add_app(app) {
            Ok(()) => {
                self.error = None;
                self.save_config();
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn start_session(&mut self, ctx: &egui::Context) {
        self.save_config();
        let policy = Policy::new(
            &self.cfg.allowed_apps,
            &process::system_roots(),
            std::env::current_exe().ok().as_deref(),
        );
        shell::write_recovery_marker(None);
        if self.cfg.hide_taskbar {
            shell::hide_taskbar();
        }
        if self.cfg.hide_desktop_icons {
            shell::hide_desktop_icons();
        }
        ctx.send_viewport_cmd(ViewportCommand::WindowLevel(WindowLevel::AlwaysOnTop));
        let session = Session::start(
            Instant::now(),
            Duration::from_secs(u64::from(self.cfg.session_minutes) * 60),
        );
        log::info!(
            "session started: {} min, apps: {:?}",
            self.cfg.session_minutes,
            self.cfg
                .allowed_apps
                .iter()
                .map(|a| &a.name)
                .collect::<Vec<_>>()
        );
        self.screen = Screen::Focus(FocusState {
            session,
            watcher: Watcher::start(
                policy,
                WatchOptions {
                    block_windowless: self.cfg.block_windowless,
                    guard_foreground: self.cfg.guard_foreground,
                },
            ),
            show_exit: false,
            exit_input: String::new(),
            toast: None,
        });
    }

    fn end_session(&mut self, ctx: &egui::Context, ended_early: bool) {
        let Screen::Focus(state) = std::mem::replace(&mut self.screen, Screen::Setup) else {
            return;
        };
        let mut state = state;
        state.watcher.stop();
        shell::restore_all();
        shell::clear_recovery_marker();
        ctx.send_viewport_cmd(ViewportCommand::WindowLevel(WindowLevel::Normal));
        let summary = state.session.summary(Instant::now(), ended_early);
        log::info!("session ended: {summary:?}");
        self.screen = Screen::Done(summary);
    }

    // ---------------------------------------------------------------- setup

    fn ui_setup(&mut self, ui: &mut egui::Ui) {
        ui.heading("be_calm");
        ui.label("この時間に使うアプリを最大3つ選んでください。それ以外のアプリは起動しても閉じられます。");
        ui.add_space(8.0);

        let mut remove: Option<usize> = None;
        for (i, app) in self.cfg.allowed_apps.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{}. {}", i + 1, app.name)).strong());
                ui.label(RichText::new(app.path.to_string_lossy()).weak().small());
                if ui.small_button("外す").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            self.cfg.remove_app(i);
            self.save_config();
        }
        let full = self.cfg.allowed_apps.len() >= MAX_ALLOWED_APPS;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!full, egui::Button::new("開いているアプリから追加"))
                .clicked()
            {
                self.picker = Some(process::windowed_apps());
            }
            if ui
                .add_enabled(!full, egui::Button::new("ファイルから選ぶ…"))
                .clicked()
            {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("実行ファイル", &["exe"])
                    .pick_file()
                {
                    self.add_app(AllowedApp::from_path(p));
                }
            }
        });

        ui.add_space(12.0);
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("時間");
            ui.add(egui::Slider::new(&mut self.cfg.session_minutes, 5..=180).suffix(" 分"));
        });
        ui.checkbox(&mut self.cfg.hide_taskbar, "タスクバーを隠す");
        ui.checkbox(
            &mut self.cfg.hide_desktop_icons,
            "デスクトップのアイコンを隠す",
        );
        ui.checkbox(
            &mut self.cfg.guard_foreground,
            "許可外のウィンドウが前に来たら最小化する",
        )
        .on_hover_text(
            "Alt+Tab や Win キーで既に開いているアプリに切り替えても、すぐ最小化されます。",
        );
        ui.checkbox(
            &mut self.cfg.block_windowless,
            "ウィンドウを持たない裏方プロセスも止める（厳格モード）",
        )
        .on_hover_text("通常はウィンドウを表示したアプリだけを止めます。OneDrive などの補助プロセスを巻き込まないためです。");
        ui.collapsing("途中で抜けるときに入力する文", |ui| {
            ui.text_edit_multiline(&mut self.cfg.exit_phrase);
        });

        ui.add_space(12.0);
        if let Some(err) = &self.error {
            ui.colored_label(Color32::from_rgb(200, 60, 60), err);
        }
        let can = self.cfg.can_start();
        if ui
            .add_enabled(
                can,
                egui::Button::new(RichText::new("集中を始める").size(20.0)),
            )
            .clicked()
        {
            self.start_session(ui.ctx());
        }
        if !can {
            ui.label(RichText::new("アプリを1つ以上選ぶと開始できます").weak());
        }
    }

    fn ui_picker(&mut self, ctx: &egui::Context) {
        let Some(list) = self.picker.clone() else {
            return;
        };
        let mut open = true;
        let mut chosen: Option<WindowedApp> = None;
        egui::Window::new("開いているアプリ")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(460.0)
            .show(ctx, |ui| {
                if list.is_empty() {
                    ui.label("ウィンドウを持つアプリが見つかりませんでした。");
                }
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        for app in &list {
                            let name = file_stem_of(&app.exe);
                            if ui
                                .button(format!("{name}  —  {}", truncate(&app.title, 60)))
                                .on_hover_text(app.exe.to_string_lossy())
                                .clicked()
                            {
                                chosen = Some(app.clone());
                            }
                        }
                    });
            });
        if let Some(app) = chosen {
            self.add_app(AllowedApp::from_path(app.exe));
            self.picker = None;
        } else if !open {
            self.picker = None;
        }
    }

    // ---------------------------------------------------------------- focus

    fn ui_focus(&mut self, ui: &mut egui::Ui) {
        let now = Instant::now();
        let ctx = ui.ctx().clone();
        let Screen::Focus(state) = &mut self.screen else {
            return;
        };

        // Drain watcher events.
        while let Some(ev) = state.watcher.try_recv() {
            match ev {
                WatchEvent::Blocked { pid, exe } => {
                    log::info!("ui: blocked pid {pid}");
                    let name = file_stem_of(&exe);
                    state.toast =
                        Some((format!("{name} を閉じました。今は集中する時間です。"), now));
                    state.session.record_block(now, exe);
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
                WatchEvent::Foreground { pid, exe } => {
                    log::info!("ui: foreground pushed back pid {pid}");
                    let name = file_stem_of(&exe);
                    state.toast = Some((format!("{name} は今は使えません。"), now));
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
                WatchEvent::KillFailed { pid, exe, reason } => {
                    log::info!("ui: kill failed pid {pid}");
                    let name = file_stem_of(&exe);
                    state.toast = Some((format!("{name} を閉じられませんでした ({reason})"), now));
                }
            }
        }

        if state.session.is_over(now) {
            self.end_session(&ctx, false);
            return;
        }

        ui.vertical_centered(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new("集中中").size(18.0));
            ui.label(
                RichText::new(format_clock(state.session.remaining(now)))
                    .size(56.0)
                    .strong(),
            );
            ui.add(egui::ProgressBar::new(state.session.progress(now)).desired_width(360.0));
        });
        ui.add_space(12.0);

        ui.label("使えるアプリ:");
        ui.horizontal_wrapped(|ui| {
            for app in &self.cfg.allowed_apps {
                if ui.button(&app.name).clicked() {
                    if let Err(e) = process::launch(&app.path) {
                        log::warn!("launch {} failed: {e}", app.path.display());
                    }
                }
            }
        });

        if let Some((msg, at)) = &state.toast {
            if now.duration_since(*at) < TOAST_TTL {
                ui.add_space(8.0);
                egui::Frame::new()
                    .fill(Color32::from_rgb(255, 240, 200))
                    .inner_margin(8.0)
                    .corner_radius(6.0)
                    .show(ui, |ui| {
                        ui.colored_label(Color32::from_rgb(90, 60, 0), msg);
                    });
            } else {
                state.toast = None;
            }
        }

        let blocks = state.session.blocks();
        if !blocks.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new(format!("ブロック {} 回", blocks.len())).weak());
        }

        ui.add_space(16.0);
        ui.separator();
        if !state.show_exit {
            if ui.small_button("途中で終える…").clicked() {
                state.show_exit = true;
            }
        } else {
            ui.label("本当に終える場合は、次の文をそのまま入力してください:");
            ui.label(RichText::new(&self.cfg.exit_phrase).strong());
            ui.text_edit_singleline(&mut state.exit_input);
            let ok = exit_challenge_passed(&state.exit_input, &self.cfg.exit_phrase);
            let quit = ui
                .horizontal(|ui| {
                    let quit = ui.add_enabled(ok, egui::Button::new("終える")).clicked();
                    if ui.button("やっぱり続ける").clicked() {
                        state.show_exit = false;
                        state.exit_input.clear();
                    }
                    quit
                })
                .inner;
            if quit {
                self.end_session(&ctx, true);
            }
        }
    }

    // ----------------------------------------------------------------- done

    fn ui_done(&mut self, ui: &mut egui::Ui, s: &SessionSummary) {
        ui.vertical_centered(|ui| {
            ui.add_space(24.0);
            ui.label(
                RichText::new(if s.ended_early {
                    "おつかれさま"
                } else {
                    "やりきりました"
                })
                .size(28.0)
                .strong(),
            );
            ui.add_space(8.0);
            ui.label(format!(
                "{} / {} 集中しました",
                format_clock(s.elapsed),
                format_clock(s.planned)
            ));
            ui.label(format!(
                "気を散らすアプリを {} 回止めました",
                s.blocked_count
            ));
            ui.add_space(24.0);
            if ui.button(RichText::new("もう一度").size(18.0)).clicked() {
                self.screen = Screen::Setup;
            }
        });
    }
}

impl eframe::App for BeCalmApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Block the window's close button during a session; route to the challenge.
        if ctx.input(|i| i.viewport().close_requested()) {
            if let Screen::Focus(state) = &mut self.screen {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                state.show_exit = true;
            }
        }

        egui::CentralPanel::default().show(ui, |ui| match &self.screen {
            Screen::Setup => self.ui_setup(ui),
            Screen::Focus(_) => self.ui_focus(ui),
            Screen::Done(s) => {
                let s = s.clone();
                self.ui_done(ui, &s)
            }
        });
        self.ui_picker(&ctx);

        if matches!(self.screen, Screen::Focus(_)) {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Screen::Focus(state) = &mut self.screen {
            state.watcher.stop();
            shell::restore_all();
            shell::clear_recovery_marker();
        }
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max_chars).collect();
        t.push('…');
        t
    }
}
