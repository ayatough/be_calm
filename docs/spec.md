# be_calm — behaviour spec

Status: MVP. This document is the source of truth for what the app does;
adjust it together with the code.

## Goal

When the PC is turned on, the user tends to open video apps or games instead
of reading, writing, studying, or programming. be_calm makes the intended
activity the path of least resistance: three chosen apps work, everything
else closes, and the visual invitations to wander (taskbar, desktop icons)
are gone for the duration of a timed session.

## Session lifecycle

1. **Setup screen.** The user picks up to `MAX_ALLOWED_APPS = 3` executables,
   a duration (5–180 min), whether to hide the taskbar and desktop icons,
   strict mode, and the early-exit phrase. Config is saved to
   `%APPDATA%\be_calm\config\config.toml` on every change.
2. **Start.** be_calm writes `session.lock` (recovery marker), hides the
   shell elements, raises its own window to always-on-top, starts the
   watcher thread, and starts the timer.
3. **Focus screen.** Remaining time, a launcher button per allowed app,
   a transient notice whenever something is blocked, and the early-exit
   control. Closing the window is refused and routed to the early-exit
   challenge instead.
4. **End.** Either the timer elapses or the user types the exit phrase
   exactly. The watcher stops, the shell is restored, the marker is
   removed, and a summary (time focused, number of blocks) is shown.

## Blocking rules (`be_calm_core::policy`, `lineage`, watcher)

A process is considered only if it appears *after* the session started.
For each new process, in order:

1. be_calm itself → allow.
2. Executable under a system root (`%SystemRoot%`) → allow. The OS spawns
   helpers there constantly; killing them breaks the desktop.
3. Executable equals an allowed app's path (case-insensitive; Microsoft
   Store apps under `WindowsApps` match on file name because their folder
   is versioned) → allow, and trust the PID.
4. Executable inside an allowed app's directory tree → allow, and trust.
5. Parent PID is trusted → allow, and trust (permission flows down the
   process tree: editor → shell → compiler).
6. Otherwise → **block**.

Trust seeding: at session start every already-running non-system process
is trusted, so an open browser's new tabs or a sync client's helpers are
never touched. System processes (explorer, shells) are *not* trusted,
because they are how new apps get launched.

Blocking means `TerminateProcess`. By default a blocked process is only
killed once it owns a visible top-level window (checked every poll), so
windowless helpers such as cloud-sync or updater processes survive.
`block_windowless = true` (strict mode) kills immediately.

Polling interval is 250 ms via a Toolhelp32 snapshot. A blocked window may
therefore flash briefly before it disappears; that is accepted, and doubles
as the "stop" signal to the user.

## Distraction removal (`platform::windows::shell`)

- **Taskbar:** previous auto-hide state is read via `SHAppBarMessage` and
  written to the recovery marker; the taskbar is then set to auto-hide and
  its windows (`Shell_TrayWnd`, `Shell_SecondaryTrayWnd`) hidden. Restore
  reverses both.
- **Desktop icons:** Explorer's own "Show desktop icons" command
  (`WM_COMMAND 0x7402` to `SHELLDLL_DefView`) is sent, which persists in
  `HKCU\...\Explorer\Advanced\HideIcons`. If the user already had icons
  hidden, be_calm leaves them alone and does not re-show them later.
- **Recovery:** `session.lock` records what was changed. On every launch,
  and via `be_calm --restore`, a leftover marker triggers a full restore. A
  panic hook does the same.

## Early exit

The window's close request is cancelled during a session. Ending early
requires typing `exit_phrase` exactly (surrounding whitespace ignored).
Default phrase: 「今は集中する時間なのに、本当に抜ける必要がある」. This is
friction, not security — the user can still end the process from Task
Manager, and that is by design.

## CLI

| flag | behaviour |
|------|-----------|
| `--restore` | restore shell, clear marker, exit |
| `--list-windows` | list processes with visible windows (what the picker shows) |
| `--dry-watch [secs]` | run the watcher with the saved config, print events; shell untouched |
| `--shell-test` | hide taskbar + icons for 6 s, restore, exit |

## Known limitations / ideas

- **Browsers:** allowing a browser allows every website. Options for later:
  window-title watching (close tabs whose title matches a blocklist), a
  `hosts`-file blocklist (needs admin), or a browser extension.
- **Launchers:** allowing Steam allows every game Steam launches (trust
  flows down the tree). Intentional; choose allowed apps accordingly.
- **UWP apps in the picker:** some show up as `ApplicationFrameHost.exe`.
  Use "ファイルから選ぶ" or pick the app while its own process has focus.
- **Notifications / Focus Assist:** no public API on Windows 11; not handled.
- **Global emergency hotkey:** not implemented; `--restore` covers recovery.
- **Already-open distractions** are not closed at session start (safety:
  unsaved work). Could become an opt-in "close these now" step.
