# CLAUDE.md — working notes for AI-assisted development

be_calm is a Windows focus-mode app in Rust. The human owner reviews the
final behaviour on their machine; the assistant implements, tests, and
pushes. Keep this file short and current.

## Layout

- `crates/be_calm_core` — OS-independent logic: config (`config.rs`), block
  policy (`policy.rs`), parent/child trust (`lineage.rs`), session timer
  (`session.rs`). Fully unit-tested; must build and pass on Linux.
- `crates/be_calm` — the binary. `app.rs` (egui screens), `fonts.rs` (CJK
  font), `platform/windows.rs` (Win32), `platform/watcher.rs` (poll loop),
  `platform/stub.rs` (non-Windows no-ops so CI can build the UI on Linux).

## Rules of the road

- Direct commits to `main`; no PRs for the single author. Every push must
  pass `cargo test`, `cargo clippy --all-targets -- -D warnings`, and
  `cargo fmt --check` (CI enforces on `windows-latest` and `ubuntu-latest`).
- Anything that can hide the taskbar must also write the recovery marker
  first, and `restore_all()` must stay idempotent. Never leave the desktop
  without a taskbar. Test with `--shell-test` and by killing the process.
- Process killing is opt-in by verdict, never by name matching alone. Read
  `docs/spec.md` before changing `policy.rs`, `lineage.rs`, or the watcher.
- Manual verification on the owner's PC uses the CLI switches
  (`--dry-watch`, `--list-windows`, `--shell-test`, `--restore`) so behaviour
  can be checked without clicking through the GUI.
- The owner uses this PC while you test. `--dry-watch` with a test config
  that omits their apps will minimize their windows (foreground guard) —
  set `guard_foreground = false` and a unique `blocked_titles` keyword in
  test configs, and restore their config afterwards.
- Don't kill processes you didn't start during a test. Use harmless
  probes (e.g. `mintty -e sleep 30`) as the "distraction".

## Gotchas found the hard way

- Never `join()` the watcher thread from the UI thread. The watcher reads the
  foreground window's title (`GetWindowTextW`), which sends `WM_GETTEXT` to
  the owning UI thread; if the UI thread is blocked in `join()` the two
  deadlock (froze the app on "終える"). Detach on stop instead.

- Already-running apps keep spawning children (browser tabs, sync helpers).
  They are trusted at session start; only *new* app launches are judged.
- Parent PIDs can point at a process that exited before the next poll, so
  "same directory as an allowed app" is also a trust rule.
- Win11 recreates the desktop icon window on relayout; use Explorer's own
  toggle command (`WM_COMMAND 0x7402`) instead of `ShowWindow` on it.
- `Start-Process -WindowStyle Hidden` makes some console apps exit at once;
  prefer `mintty` as a windowed probe when testing on the owner's PC.
- `SetForegroundWindow` / `AppActivate` from a script do not reliably move
  focus (foreground lock). To test the foreground guard, start the probe
  window *before* `--dry-watch`, then click into it with a real input tool.
- The owner may have a release build running while you test a debug build.
  Self-detection is by file name so the two never fight; also never
  `Stop-Process -Name be_calm` blindly — check the path first.
