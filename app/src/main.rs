
#![windows_subsystem = "windows"]

mod flyout;
mod gip;
mod hid;
mod icon;
mod link;
mod protocol;
mod state;
mod store;
mod theme;
mod tray;

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, SetProcessWorkingSetSize};
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_SINGLETHREADED};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, FindWindowW, GetMessageW, LoadCursorW,
    PostMessageW, PostQuitMessage,
    KillTimer, RegisterClassW, RegisterWindowMessageW, SetTimer, TranslateMessage, CW_USEDEFAULT,
    IDC_ARROW, MSG, WM_ACTIVATE, WM_LBUTTONUP, WM_DESTROY, WM_DEVICECHANGE, WM_DPICHANGED,
    WM_ERASEBKGND, WM_KEYDOWN, WM_MOUSEMOVE, WM_PAINT, WM_SETTINGCHANGE, WM_TIMER,
    WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_OVERLAPPED, WS_POPUP,
};

/// Not exported by the windows crate at this version.
const WM_MOUSELEAVE: u32 = 0x02A3;

const TIMER_TICK: usize = 1;
const TIMER_SETTLE: usize = 2;
const TIMER_POLL: usize = 3;
const TIMER_TRAY: usize = 4;

const TRAY_RETRY_MIN_MS: u32 = 2_000;
const TRAY_RETRY_MAX_MS: u32 = 30_000;
static TRAY_RETRY_MS: AtomicU32 = AtomicU32::new(TRAY_RETRY_MIN_MS);

static PENDING_SHOW: AtomicBool = AtomicBool::new(false);

fn ensure_tray(hwnd: HWND) {
    if tray_mut().is_some_and(|t| t.is_shown()) {
        return;
    }
    let added = tray_mut().map(|t| t.install()).unwrap_or(false);
    unsafe {
        if added {
            TRAY_RETRY_MS.store(TRAY_RETRY_MIN_MS, Ordering::Relaxed);
            KillTimer(hwnd, TIMER_TRAY).ok();
            if let Some(t) = tray_mut() {
                t.update(&tooltip());
            }
            if PENDING_SHOW.swap(false, Ordering::Relaxed) {
                open_panel();
            }
        } else {
            let wait = TRAY_RETRY_MS.load(Ordering::Relaxed);
            SetTimer(hwnd, TIMER_TRAY, wait, None);
            TRAY_RETRY_MS.store((wait * 2).min(TRAY_RETRY_MAX_MS), Ordering::Relaxed);
        }
    }
}

unsafe fn show_panel_when_ready(hwnd: HWND) {
    if tray_mut().is_some_and(|t| t.is_shown()) {
        open_panel();
        return;
    }
    PENDING_SHOW.store(true, Ordering::Relaxed);
    TRAY_RETRY_MS.store(TRAY_RETRY_MIN_MS, Ordering::Relaxed);
    ensure_tray(hwnd);
}

unsafe fn open_panel() {
    let fly = flyout_hwnd();
    if !fly.is_invalid() && !flyout::is_visible(fly) {
        toggle_flyout();
    }
}

fn trim_working_set() {
    unsafe {
        let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

static mut TRAY: Option<tray::Tray> = None;
static FLYOUT_HWND: AtomicIsize = AtomicIsize::new(0);

fn flyout_hwnd() -> HWND {
    HWND(FLYOUT_HWND.load(Ordering::Relaxed) as *mut _)
}

static HOST_HWND: AtomicIsize = AtomicIsize::new(0);

fn host_hwnd() -> HWND {
    HWND(HOST_HWND.load(Ordering::Relaxed) as *mut _)
}

static mut TASKBAR_CREATED: u32 = 0;
static mut WM_SHOW_PANEL: u32 = 0;

fn tray_mut() -> Option<&'static mut tray::Tray> {
    unsafe { (&raw mut TRAY).as_mut().and_then(|t| t.as_mut()) }
}

fn tooltip() -> String {
    let devices = state::snapshot();
    let Some(d) = devices.iter().find(|d| d.connected).or(devices.first()) else {
        return "No controller connected".into();
    };
    if !d.connected {
        return format!("{} \u{2014} disconnected", d.name);
    }
    match (d.percent(), d.battery) {
        (Some(p), Some(s)) => {
            let charge = if d.is_charging() { ", charging" } else { "" };
            format!(
                "{} \u{2014} {}%{}\n{}",
                d.name,
                p.round() as i32,
                charge,
                state::describe_age(s.age())
            )
        }
        _ => format!("{} \u{2014} open the panel to read battery", d.name),
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        tray::WM_TRAY => {
            let (event, _x, _y) = tray::tray_event(w, l);
            const NIN_SELECT: u32 = 0x0400;
            const NIN_KEYSELECT: u32 = 0x0401;
            const WM_CONTEXTMENU: u32 = 0x007B;
            // With NOTIFYICON_VERSION_4 the shell sends both NIN_SELECT and
            // WM_LBUTTONUP for one click, so only the NIN_* messages are taken.
            match event {
                NIN_SELECT | NIN_KEYSELECT => toggle_flyout(),
                WM_CONTEXTMENU | 0x0205 /* WM_RBUTTONUP */ => open_settings(),
                _ => {}
            }
            LRESULT(0)
        }
        state::WM_STATE_CHANGED => {
            if let Some(t) = tray_mut() {
                t.update(&tooltip());
            }
            let fly = flyout_hwnd();
            if !fly.is_invalid() && flyout::is_visible(fly) {
                flyout::resize_to_content(fly);
                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(fly, None, false);
            }
            LRESULT(0)
        }
        WM_TIMER if w.0 == TIMER_POLL => {
            gip::request_refresh();
            hid::request_refresh();
            LRESULT(0)
        }
        WM_TIMER if w.0 == TIMER_TRAY => {
            ensure_tray(hwnd);
            LRESULT(0)
        }
        WM_TIMER if w.0 == TIMER_SETTLE => {
            KillTimer(hwnd, TIMER_SETTLE).ok();
            trim_working_set();
            LRESULT(0)
        }
        WM_TIMER if w.0 == TIMER_TICK => {
            gip::reconcile_connected();
            ensure_tray(hwnd);
            if let Some(t) = tray_mut() {
                t.update(&tooltip());
            }
            let fly = flyout_hwnd();
            if fly.is_invalid() || !flyout::is_visible(fly) {
                trim_working_set();
            }
            LRESULT(0)
        }
        WM_DEVICECHANGE => {
            gip::reconcile_connected();
            LRESULT(1)
        }
        WM_SETTINGCHANGE | WM_DPICHANGED => {
            if let Some(t) = tray_mut() {
                t.update(&tooltip());
            }
            LRESULT(0)
        }
        // Explorer restarted and took the icon with it.
        _ if msg == TASKBAR_CREATED && TASKBAR_CREATED != 0 => {
            if let Some(t) = tray_mut() {
                t.mark_lost();
            }
            TRAY_RETRY_MS.store(TRAY_RETRY_MIN_MS, Ordering::Relaxed);
            ensure_tray(hwnd);
            LRESULT(0)
        }
        _ if msg == WM_SHOW_PANEL && WM_SHOW_PANEL != 0 => {
            show_panel_when_ready(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            if let Some(t) = tray_mut() {
                t.remove();
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}

unsafe extern "system" fn flyout_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => flyout::on_paint(hwnd),
        WM_ERASEBKGND => LRESULT(1), // fully repainted every time
        WM_ACTIVATE => {
            if (w.0 & 0xFFFF) == 0 {
                flyout::hide(hwnd);
                KillTimer(host_hwnd(), TIMER_POLL).ok();
                trim_working_set();
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, y) = ((l.0 & 0xFFFF) as i16 as i32, ((l.0 >> 16) & 0xFFFF) as i16 as i32);
            let changed = match flyout::page() {
                flyout::Page::Settings => flyout::set_hover(flyout::settings_row_at(hwnd, x, y)),
                flyout::Page::Battery => {
                    let footer = flyout::set_footer_hover(flyout::footer_hover_at(hwnd, x, y));
                    let chart = flyout::set_chart_hover(flyout::chart_hover_at(hwnd, x, y));
                    footer || chart
                }
            };
            if changed {
                let _ = InvalidateRect(hwnd, None, false);
            }
            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            let _ = TrackMouseEvent(&mut tme);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            let a = flyout::set_hover(None);
            let b = flyout::set_footer_hover(None);
            let c = flyout::set_chart_hover(None);
            if a || b || c {
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = ((l.0 & 0xFFFF) as i16 as i32, ((l.0 >> 16) & 0xFFFF) as i16 as i32);
            let gear = flyout::gear_rect(hwnd);
            if x >= gear.left && x <= gear.right && y >= gear.top && y <= gear.bottom {
                flyout::set_page(match flyout::page() {
                    flyout::Page::Battery => flyout::Page::Settings,
                    flyout::Page::Settings => flyout::Page::Battery,
                });
                flyout::resize_to_content(hwnd);
                let _ = InvalidateRect(hwnd, None, false);
                return LRESULT(0);
            }
            if flyout::page() == flyout::Page::Settings {
                match flyout::settings_row_at(hwnd, x, y) {
                    Some(flyout::ROW_REFRESH) => {
                        gip::request_refresh();
                        hid::request_refresh();
                        flyout::set_page(flyout::Page::Battery);
                        flyout::resize_to_content(hwnd);
                        let _ = InvalidateRect(hwnd, None, false);
                    }
                    Some(flyout::ROW_AUTOSTART) => {
                        theme::set_autostart(!theme::autostart_enabled());
                        let _ = InvalidateRect(hwnd, None, false);
                    }
                    Some(flyout::ROW_QUIT) => {
                        if let Some(t) = tray_mut() {
                            t.remove();
                        }
                        PostQuitMessage(0);
                    }
                    _ => {}
                }
            }
            LRESULT(0)
        }
        WM_KEYDOWN if w.0 == 0x1B => {
            flyout::hide(hwnd);
            KillTimer(host_hwnd(), TIMER_POLL).ok();
            trim_working_set();
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}

unsafe fn toggle_flyout() {
    let fly = flyout_hwnd();
    if fly.is_invalid() {
        return;
    }
    if flyout::is_visible(fly) {
        flyout::hide(fly);
        return;
    }
    let anchor = tray_mut()
        .and_then(|t| t.icon_rect())
        .unwrap_or(RECT { left: 0, top: 0, right: 0, bottom: 0 });
    flyout::show(fly, anchor);
    SetTimer(host_hwnd(), TIMER_POLL, 3_000, None);
}

unsafe fn open_settings() {
    let fly = flyout_hwnd();
    if fly.is_invalid() {
        return;
    }
    flyout::set_page(flyout::Page::Settings);
    if flyout::is_visible(fly) {
        flyout::resize_to_content(fly);
        let _ = InvalidateRect(fly, None, false);
    } else {
        toggle_flyout();
    }
}

fn main() {
    unsafe {
        let show_panel = RegisterWindowMessageW(w!("WolverineBatteryShowPanel"));
        let _mutex = CreateMutexW(None, true, w!("Local\\WolverineBattery.SingleInstance"));
        if windows::Win32::Foundation::GetLastError()
            == windows::Win32::Foundation::ERROR_ALREADY_EXISTS
        {
            if let Ok(running) = FindWindowW(w!("WolverineBatteryHost"), PCWSTR::null()) {
                let _ = PostMessageW(running, show_panel, WPARAM(0), LPARAM(0));
            }
            return;
        }
        WM_SHOW_PANEL = show_panel;

        store::load();
        theme::repair_autostart();
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = RoInitialize(RO_INIT_SINGLETHREADED);

        let hinst = GetModuleHandleW(None).expect("module handle");

        let main_class = w!("WolverineBatteryHost");
        let arrow = LoadCursorW(None, IDC_ARROW).unwrap_or_default();

        let _ = RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: main_class,
            hCursor: arrow,
            ..Default::default()
        });
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            main_class,
            w!("Wolverine Battery"),
            WS_OVERLAPPED,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            0,
            0,
            None,
            None,
            hinst,
            None,
        )
        .expect("host window");

        let flyout_class = w!("WolverineBatteryFlyout");
        let _ = RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(flyout_proc),
            hInstance: hinst.into(),
            lpszClassName: flyout_class,
            hCursor: arrow,
            ..Default::default()
        });
        let flyout_window = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            flyout_class,
            w!("Wolverine Battery"),
            WS_POPUP,
            0,
            0,
            10,
            10,
            None,
            None,
            hinst,
            None,
        )
        .expect("flyout window");
        FLYOUT_HWND.store(flyout_window.0 as isize, Ordering::Relaxed);

        TASKBAR_CREATED = RegisterWindowMessageW(w!("TaskbarCreated"));
        state::set_ui_window(hwnd);
        HOST_HWND.store(hwnd.0 as isize, Ordering::Relaxed);

        let mut t = tray::Tray::new(hwnd);
        t.update(&tooltip());
        TRAY = Some(t);
        PENDING_SHOW.store(!theme::launched_at_logon(), Ordering::Relaxed);
        ensure_tray(hwnd);

        gip::register();
        hid::request_refresh();

        SetTimer(hwnd, TIMER_TICK, 30_000, None);
        SetTimer(hwnd, TIMER_SETTLE, 3_000, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
