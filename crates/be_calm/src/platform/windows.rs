//! Win32 integration: process table, termination, window enumeration,
//! and shell tweaks (taskbar / desktop icons).

use super::{Foreground, WindowedApp};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use windows::core::{w, BOOL, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, HWND, LPARAM, WPARAM};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, TerminateProcess, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
};
use windows::Win32::UI::Shell::{SHAppBarMessage, ShellExecuteW, APPBARDATA};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, FindWindowW, GetForegroundWindow, GetWindow, GetWindowLongPtrW,
    GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, SendMessageW, ShowWindow,
    GWL_EXSTYLE, GW_OWNER, SW_HIDE, SW_MINIMIZE, SW_SHOW, SW_SHOWNORMAL, WM_COMMAND,
    WS_EX_TOOLWINDOW,
};

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub mod process {
    use super::*;

    #[derive(Debug, Clone)]
    pub struct ProcInfo {
        pub pid: u32,
        pub parent: u32,
        pub exe: Option<PathBuf>,
    }

    /// Full image path of a process, if we are allowed to query it.
    pub fn exe_path(pid: u32) -> Option<PathBuf> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let r = QueryFullProcessImageNameW(
                h,
                PROCESS_NAME_WIN32,
                PWSTR(buf.as_mut_ptr()),
                &mut len,
            );
            let _ = CloseHandle(h);
            r.ok()?;
            Some(PathBuf::from(String::from_utf16_lossy(
                &buf[..len as usize],
            )))
        }
    }

    pub fn list() -> Vec<ProcInfo> {
        let mut out = Vec::new();
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
                log::error!("CreateToolhelp32Snapshot failed");
                return out;
            };
            let mut pe = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            if Process32FirstW(snap, &mut pe).is_ok() {
                loop {
                    let pid = pe.th32ProcessID;
                    if pid != 0 {
                        out.push(ProcInfo {
                            pid,
                            parent: pe.th32ParentProcessID,
                            exe: exe_path(pid),
                        });
                    }
                    if Process32NextW(snap, &mut pe).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snap);
        }
        out
    }

    /// Kill a process. A process that is already gone counts as success.
    pub fn terminate(pid: u32) -> Result<(), String> {
        unsafe {
            let h = match OpenProcess(PROCESS_TERMINATE, false, pid) {
                Ok(h) => h,
                // ERROR_INVALID_PARAMETER: no such process (it already exited).
                Err(e) if e.code() == ERROR_INVALID_PARAMETER.to_hresult() => return Ok(()),
                Err(e) => return Err(e.to_string()),
            };
            let r = TerminateProcess(h, 1).map_err(|e| e.to_string());
            let _ = CloseHandle(h);
            r
        }
    }

    /// Distinct executables that currently own a visible, titled, top-level window.
    pub fn windowed_apps() -> Vec<WindowedApp> {
        let mut apps: Vec<WindowedApp> = Vec::new();
        unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let apps = unsafe { &mut *(lparam.0 as *mut Vec<WindowedApp>) };
            unsafe {
                if !IsWindowVisible(hwnd).as_bool() {
                    return true.into();
                }
                if GetWindow(hwnd, GW_OWNER).is_ok() {
                    return true.into(); // owned popup, not a main window
                }
                let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
                if ex & WS_EX_TOOLWINDOW.0 != 0 {
                    return true.into();
                }
                let mut title = [0u16; 512];
                let n = GetWindowTextW(hwnd, &mut title);
                if n <= 0 {
                    return true.into();
                }
                let title = String::from_utf16_lossy(&title[..n as usize]);
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if pid == 0 || pid == std::process::id() {
                    return true.into();
                }
                let Some(exe) = exe_path(pid) else {
                    return true.into();
                };
                if apps.iter().any(|a| a.exe == exe) {
                    return true.into();
                }
                apps.push(WindowedApp { pid, title, exe });
            }
            true.into()
        }
        unsafe {
            let _ = EnumWindows(Some(cb), LPARAM(&mut apps as *mut _ as isize));
        }
        apps.sort_by_key(|a| a.title.to_lowercase());
        apps
    }

    /// PIDs that currently own at least one visible top-level window.
    pub fn pids_with_visible_windows() -> HashSet<u32> {
        let mut pids: HashSet<u32> = HashSet::new();
        unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let pids = unsafe { &mut *(lparam.0 as *mut HashSet<u32>) };
            unsafe {
                if !IsWindowVisible(hwnd).as_bool() {
                    return true.into();
                }
                let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
                if ex & WS_EX_TOOLWINDOW.0 != 0 {
                    return true.into();
                }
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if pid != 0 {
                    pids.insert(pid);
                }
            }
            true.into()
        }
        unsafe {
            let _ = EnumWindows(Some(cb), LPARAM(&mut pids as *mut _ as isize));
        }
        pids
    }

    /// The foreground window and the PID that really owns it. UWP/Store apps
    /// are hosted by `ApplicationFrameHost.exe`; for those we look for the
    /// app's `Windows.UI.Core.CoreWindow` child to find the actual process.
    pub fn foreground() -> Option<Foreground> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.0.is_null() {
                return None;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == 0 {
                return None;
            }
            let mut exe = exe_path(pid);
            let is_frame_host = exe
                .as_ref()
                .map(|e| {
                    be_calm_core::config::file_name_of(e)
                        .eq_ignore_ascii_case("ApplicationFrameHost.exe")
                })
                .unwrap_or(false);
            if is_frame_host {
                if let Ok(core) =
                    FindWindowExW(Some(hwnd), None, w!("Windows.UI.Core.CoreWindow"), None)
                {
                    if !core.0.is_null() {
                        let mut app_pid = 0u32;
                        GetWindowThreadProcessId(core, Some(&mut app_pid));
                        if app_pid != 0 {
                            pid = app_pid;
                            exe = exe_path(pid);
                        }
                    }
                }
            }
            Some(Foreground {
                hwnd: hwnd.0 as isize,
                pid,
                exe,
            })
        }
    }

    /// Minimize a window (by raw handle from `foreground()`).
    pub fn minimize(hwnd: isize) {
        unsafe {
            let _ = ShowWindow(HWND(hwnd as *mut _), SW_MINIMIZE);
        }
    }

    /// Launch an app the way the shell would (works for Store apps too).
    pub fn launch(path: &Path) -> Result<(), String> {
        let wide = to_wide(&path.to_string_lossy());
        let h = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                PCWSTR(wide.as_ptr()),
                None,
                None,
                SW_SHOWNORMAL,
            )
        };
        // Per docs, values > 32 mean success.
        if h.0 as isize > 32 {
            Ok(())
        } else {
            Err(format!("ShellExecute failed with code {}", h.0 as isize))
        }
    }

    /// Directories whose contents are always allowed.
    pub fn system_roots() -> Vec<PathBuf> {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        vec![PathBuf::from(root)]
    }
}

pub mod shell {
    use super::*;

    const ABM_GETSTATE: u32 = 4;
    const ABM_SETSTATE: u32 = 10;
    const ABS_AUTOHIDE: u32 = 1;

    fn taskbars() -> Vec<HWND> {
        let mut v = Vec::new();
        unsafe {
            if let Ok(h) = FindWindowW(w!("Shell_TrayWnd"), None) {
                if !h.0.is_null() {
                    v.push(h);
                }
            }
            // Secondary monitors.
            let mut prev: Option<HWND> = None;
            while let Ok(h) = FindWindowExW(None, prev, w!("Shell_SecondaryTrayWnd"), None) {
                if h.0.is_null() {
                    break;
                }
                v.push(h);
                prev = Some(h);
            }
        }
        v
    }

    /// Every `SHELLDLL_DefView` window (the desktop icon host). Depending on
    /// wallpaper mode it lives under `Progman` or under one of the `WorkerW`
    /// windows, and stale empty ones can coexist, so we take all of them.
    fn desktop_icon_views() -> Vec<HWND> {
        let mut v = Vec::new();
        unsafe {
            if let Ok(progman) = FindWindowW(w!("Progman"), None) {
                if let Ok(d) = FindWindowExW(Some(progman), None, w!("SHELLDLL_DefView"), None) {
                    if !d.0.is_null() {
                        v.push(d);
                    }
                }
            }
            let mut prev: Option<HWND> = None;
            while let Ok(wk) = FindWindowExW(None, prev, w!("WorkerW"), None) {
                if wk.0.is_null() {
                    break;
                }
                if let Ok(d) = FindWindowExW(Some(wk), None, w!("SHELLDLL_DefView"), None) {
                    if !d.0.is_null() {
                        v.push(d);
                    }
                }
                prev = Some(wk);
            }
        }
        v
    }

    fn appbar_state() -> u32 {
        let mut d = APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            ..Default::default()
        };
        unsafe { SHAppBarMessage(ABM_GETSTATE, &mut d) as u32 }
    }

    fn set_appbar_state(state: u32) {
        let mut d = APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            lParam: LPARAM(state as isize),
            ..Default::default()
        };
        unsafe {
            SHAppBarMessage(ABM_SETSTATE, &mut d);
        }
    }

    /// Hide the taskbar: switch it to auto-hide (so maximized windows get the
    /// space back) and hide its window. The previous auto-hide state is saved
    /// in the recovery marker so it can be restored even after a crash.
    pub fn hide_taskbar() {
        let prev = appbar_state();
        write_recovery_marker(Some(prev));
        set_appbar_state(prev | ABS_AUTOHIDE);
        for h in taskbars() {
            unsafe {
                let _ = ShowWindow(h, SW_HIDE);
            }
        }
        log::info!("taskbar hidden (previous appbar state {prev})");
    }

    pub fn show_taskbar(previous_state: u32) {
        set_appbar_state(previous_state);
        for h in taskbars() {
            unsafe {
                let _ = ShowWindow(h, SW_SHOW);
            }
        }
    }

    /// Explorer's own "Show desktop icons" toggle (View menu). Sending this
    /// command is exactly what the context-menu item does, so Explorer keeps
    /// the state consistently (unlike hiding the icon window ourselves, which
    /// Explorer may undo on the next relayout). It persists in the registry,
    /// which is what lets us restore it after a crash.
    const CMD_TOGGLE_DESKTOP_ICONS: usize = 0x7402;

    /// Current value of `HKCU\...\Explorer\Advanced\HideIcons` (1 = hidden).
    fn desktop_icons_hidden() -> bool {
        let mut val: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        let key = to_wide(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced");
        let name = to_wide("HideIcons");
        let r = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(key.as_ptr()),
                PCWSTR(name.as_ptr()),
                RRF_RT_REG_DWORD,
                None,
                Some(&mut val as *mut u32 as *mut _),
                Some(&mut size),
            )
        };
        r.is_ok() && val != 0
    }

    fn toggle_desktop_icons() {
        let views = desktop_icon_views();
        if views.is_empty() {
            log::warn!("desktop icon view not found");
            return;
        }
        unsafe {
            SendMessageW(
                views[0],
                WM_COMMAND,
                Some(WPARAM(CMD_TOGGLE_DESKTOP_ICONS)),
                None,
            );
        }
    }

    pub fn hide_desktop_icons() {
        if desktop_icons_hidden() {
            log::info!("desktop icons were already hidden by the user; leaving as is");
            return;
        }
        write_recovery_marker_icons(true);
        toggle_desktop_icons();
        log::info!("desktop icons hidden");
    }

    pub fn show_desktop_icons() {
        // Only undo what we did: if the user hides icons themselves, respect it.
        if read_recovery_marker_icons() && desktop_icons_hidden() {
            toggle_desktop_icons();
            log::info!("desktop icons shown");
        }
    }

    /// Idempotent: safe to call even if nothing was hidden. Uses the recovery
    /// marker (if any) to restore the taskbar's previous auto-hide setting.
    pub fn restore_all() {
        let prev = read_recovery_marker().unwrap_or(0);
        show_taskbar(prev);
        show_desktop_icons();
        log::info!("shell restored (appbar state {prev})");
    }

    fn read_recovery_marker_icons() -> bool {
        std::fs::read_to_string(marker_path())
            .map(|s| s.lines().any(|l| l.trim() == "icons_hidden = true"))
            .unwrap_or(false)
    }

    fn write_recovery_marker_icons(hidden: bool) {
        let state = read_recovery_marker().unwrap_or(0);
        let p = marker_path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(
            p,
            format!("taskbar_state = {state}\nicons_hidden = {hidden}\n"),
        );
    }

    fn marker_path() -> PathBuf {
        be_calm_core::config::data_dir().join("session.lock")
    }

    /// Record that the shell is (about to be) modified. `taskbar_state` is the
    /// appbar state to restore; `None` keeps whatever was recorded before.
    pub fn write_recovery_marker(taskbar_state: Option<u32>) {
        let p = marker_path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let state = taskbar_state.or_else(read_recovery_marker).unwrap_or(0);
        let icons = read_recovery_marker_icons();
        let _ = std::fs::write(
            p,
            format!("taskbar_state = {state}\nicons_hidden = {icons}\n"),
        );
    }

    fn read_recovery_marker() -> Option<u32> {
        let s = std::fs::read_to_string(marker_path()).ok()?;
        s.lines()
            .find_map(|l| l.strip_prefix("taskbar_state = "))
            .and_then(|v| v.trim().parse().ok())
    }

    pub fn clear_recovery_marker() {
        let _ = std::fs::remove_file(marker_path());
    }

    pub fn recovery_marker_exists() -> bool {
        marker_path().exists()
    }
}
