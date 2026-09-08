//! Notification-area icon.

use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconGetRect, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP,
    NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICONDATAW, NOTIFYICONIDENTIFIER,
    NOTIFYICON_VERSION_4,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON};
use windows::Win32::UI::HiDpi::GetDpiForWindow;

use crate::icon::{self, IconSpec};
use crate::state;

pub const WM_TRAY: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 2;
const ICON_ID: u32 = 1;

pub struct Tray {
    hwnd: HWND,
    icon: Option<HICON>,
    shown: bool,
    last: Option<IconSpec>,
}

impl Tray {
    pub fn new(hwnd: HWND) -> Self {
        Tray {
            hwnd,
            icon: None,
            shown: false,
            last: None,
        }
    }

    pub fn is_shown(&self) -> bool {
        self.shown
    }

    pub fn mark_lost(&mut self) {
        self.shown = false;
    }

    fn base(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: ICON_ID,
            ..Default::default()
        }
    }

    pub fn update(&mut self, tooltip: &str) {
        let dpi = unsafe { GetDpiForWindow(self.hwnd) }.max(96);
        let size = (16 * dpi / 96).clamp(16, 32);
        let devices = state::snapshot();
        let active = devices.iter().find(|d| d.connected).or(devices.first());

        let spec = IconSpec {
            size,
            fill: active
                .filter(|d| d.connected)
                .and_then(|d| d.percent())
                .map(|p| p.round().clamp(0.0, 100.0) as u8),
            charging: active.map(|d| d.is_charging()).unwrap_or(false),
            light_taskbar: crate::theme::taskbar_is_light(),
        };

        if self.last != Some(spec) || !self.shown {
            if let Some(h) = icon::render(spec) {
                let old = self.icon.replace(h);
                self.last = Some(spec);
                let mut nid = self.base();
                nid.uFlags = NIF_ICON;
                nid.hIcon = h;
                if self.shown {
                    unsafe { let _ = Shell_NotifyIconW(NIM_MODIFY, &nid); }
                }
                if let Some(o) = old {
                    unsafe { let _ = DestroyIcon(o); }
                }
            }
        }

        let mut nid = self.base();
        nid.uFlags = NIF_TIP | NIF_SHOWTIP;
        let mut buf: Vec<u16> = tooltip.encode_utf16().take(127).collect();
        buf.push(0);
        nid.szTip[..buf.len()].copy_from_slice(&buf);
        if self.shown {
            unsafe { let _ = Shell_NotifyIconW(NIM_MODIFY, &nid); }
        }
    }

    #[must_use]
    pub fn install(&mut self) -> bool {
        self.shown = false;
        let mut nid = self.base();
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
        nid.uCallbackMessage = WM_TRAY;
        if let Some(h) = self.icon {
            nid.hIcon = h;
        }
        unsafe {
            if !Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
                return false;
            }
            nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
        }
        self.shown = true;
        true
    }

    pub fn remove(&mut self) {
        if self.shown {
            unsafe { let _ = Shell_NotifyIconW(NIM_DELETE, &self.base()); }
            self.shown = false;
        }
        if let Some(h) = self.icon.take() {
            unsafe { let _ = DestroyIcon(h); }
        }
    }

    pub fn icon_rect(&self) -> Option<RECT> {
        let id = NOTIFYICONIDENTIFIER {
            cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: self.hwnd,
            uID: ICON_ID,
            ..Default::default()
        };
        unsafe { Shell_NotifyIconGetRect(&id).ok() }
    }

}

/// Unpack the coordinates the shell sends with version-4 callbacks.
pub fn tray_event(wparam: WPARAM, lparam: LPARAM) -> (u32, i32, i32) {
    let x = (wparam.0 & 0xFFFF) as i16 as i32;
    let y = ((wparam.0 >> 16) & 0xFFFF) as i16 as i32;
    let msg = (lparam.0 & 0xFFFF) as u32;
    (msg, x, y)
}
