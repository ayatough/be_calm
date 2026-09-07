# be_calm

Focus mode for Windows. Pick up to three apps, start a timer, and everything
else you try to open gets closed on the spot. The taskbar and desktop icons go
away for the duration so there is nothing to wander off to.

Written in Rust (egui + Win32). No admin rights, no drivers, no kernel hooks.

## What it does

- **Allowlist of three.** Choose apps from the ones currently open or pick an
  `.exe`. During a session any *new* app that shows a window and isn't on the
  list (or launched by something on the list) is terminated, with a short
  notice in the be_calm window.
- **Fewer distractions.** Taskbar hidden (switched to auto-hide so maximized
  windows reclaim the space) and desktop icons hidden. Both are restored when
  the session ends — or on the next launch / `be_calm --restore` if the app
  ever dies mid-session.
- **Friction to quit.** The window can't simply be closed while a session is
  running; ending early requires typing a sentence you chose in advance.
- **Already-open apps stay out of the way.** They are not closed (unsaved
  work is safe), but if one of them comes to the front — Alt+Tab, Win key,
  taskbar — it is minimized right away. Background helpers without a window
  are left alone unless you enable strict mode.

## Usage

```
cargo run --release
```

Command line switches (all exit without showing the GUI):

| flag | purpose |
|------|---------|
| `--restore` | put the taskbar / desktop icons back and clear the recovery marker |
| `--list-windows` | print the apps that currently own a visible window |
| `--dry-watch [secs]` | run the process watcher with the saved config and print what it blocks; does not touch the shell |
| `--shell-test` | hide taskbar + icons for six seconds, then restore |

Config and log live in `%APPDATA%\be_calm\config\`.

## Development

```
cargo test            # core logic, runs on any OS
cargo clippy --all-targets
cargo fmt --check
```

See [docs/spec.md](docs/spec.md) for the behaviour spec and known limitations,
and [CLAUDE.md](CLAUDE.md) for the working agreement used when developing
with an AI assistant.

## License

MIT
