//! The panel that appears when the tray icon is clicked.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LRESULT, POINT, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateDIBSection, CreateFontW,
    CreatePen, CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, GdiFlush,
    LineTo, MonitorFromRect, MoveToEx, Polyline, RoundRect, SelectObject, SetBkMode, SetTextColor,
    GetDC, GetTextFaceW, ReleaseDC,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CLEARTYPE_QUALITY, DIB_RGB_COLORS, DRAW_TEXT_FORMAT,
    DT_CENTER, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE,
    DT_VCENTER, DT_WORDBREAK, FW_NORMAL, FW_SEMIBOLD, HBITMAP, HBRUSH, HDC, HFONT,
    MONITOR_DEFAULTTONEAREST, PAINTSTRUCT, PS_SOLID, SRCCOPY, TRANSPARENT,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetWindowRect, IsWindowVisible, SetForegroundWindow, SetWindowPos, ShowWindow,
    SystemParametersInfoW, HWND_TOPMOST, SPI_GETWORKAREA, SWP_NOACTIVATE, SWP_NOZORDER,
    SW_HIDE, SW_SHOW, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
};

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use crate::state::{self, Device};

const SP_1: i32 = 4;
const SP_2: i32 = 8;
const SP_3: i32 = 12;
const SP_4: i32 = 16;
const SP_5: i32 = 20;
const SP_6: i32 = 24;

const TX_DISPLAY: i32 = 36;
const TX_TITLE: i32 = 15;
const TX_BODY: i32 = 13;
const TX_META: i32 = 11;

const GL_LARGE: i32 = 20;
const GL_MEDIUM: i32 = 16;
const GL_SMALL: i32 = 12;

pub const WIDTH_DIP: i32 = 340;
const PAD: i32 = SP_5;
const PAD_BOTTOM: i32 = SP_2;
const CARD_DIP: i32 = 288;
const EMPTY_DIP: i32 = 88;

const CHART_GUTTER: i32 = 32;
const CHART_H: i32 = 56;
const AXIS_H: i32 = SP_5;

const HEAD_H: i32 = 36;
const LEVEL_H: i32 = 44;
const FOOT_H: i32 = SP_5;

fn icon_font() -> PCWSTR {
    static HAS_FLUENT: OnceLock<bool> = OnceLock::new();
    let fluent = *HAS_FLUENT.get_or_init(|| unsafe {
        let dc = GetDC(None);
        if dc.is_invalid() {
            return false;
        }
        let f = CreateFontW(
            -12, 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0, 0,
            Default::default(), Default::default(), Default::default(), Default::default(),
            w!("Segoe Fluent Icons"),
        );
        let old = SelectObject(dc, f);
        let mut name = [0u16; 64];
        let n = GetTextFaceW(dc, Some(&mut name)) as usize;
        SelectObject(dc, old);
        let _ = DeleteObject(f);
        ReleaseDC(None, dc);
        String::from_utf16_lossy(&name[..n.saturating_sub(1)])
            .eq_ignore_ascii_case("Segoe Fluent Icons")
    });
    if fluent {
        w!("Segoe Fluent Icons")
    } else {
        w!("Segoe MDL2 Assets")
    }
}

const GLYPH_GAMEPAD: &str = "\u{E7FC}";
const GLYPH_BOLT: &str = "\u{E945}";
const GLYPH_GEAR: &str = "\u{E713}";
const GLYPH_BACK: &str = "\u{E72B}";
const GLYPH_CHECK: &str = "\u{E73E}";
const GLYPH_INFO: &str = "\u{E946}";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Battery,
    Settings,
}

pub const ROW_REFRESH: usize = 0;
pub const ROW_AUTOSTART: usize = 1;
pub const ROW_QUIT: usize = 2;
const ROW_COUNT: usize = 3;
const ROW_DIP: i32 = 40;
const HEADER_DIP: i32 = 40;
const SETTINGS_DIP: i32 = PAD + HEADER_DIP + ROW_COUNT as i32 * ROW_DIP + PAD;

static PAGE: AtomicI32 = AtomicI32::new(0);
static HOVER: AtomicI32 = AtomicI32::new(-1);

pub fn page() -> Page {
    if PAGE.load(Ordering::Relaxed) == 1 {
        Page::Settings
    } else {
        Page::Battery
    }
}

pub fn set_page(p: Page) {
    PAGE.store(if p == Page::Settings { 1 } else { 0 }, Ordering::Relaxed);
    HOVER.store(-1, Ordering::Relaxed);
}

pub fn set_hover(row: Option<usize>) -> bool {
    let want = row.map(|r| r as i32).unwrap_or(-1);
    HOVER.swap(want, Ordering::Relaxed) != want
}

static FOOTERS: Mutex<Vec<[i32; 4]>> = Mutex::new(Vec::new());
/// Card whose info glyph the pointer is over, or -1.
static FOOTER_HOVER: AtomicI32 = AtomicI32::new(-1);

pub fn footer_hover_at(hwnd: HWND, x: i32, y: i32) -> Option<i32> {
    let s = |v: i32| scale(v, dpi_for_window(hwnd));
    let footers = FOOTERS.lock().ok()?;
    for (i, [_, t, r, b]) in footers.iter().enumerate() {
        if y >= *t && y <= *b && x >= *r - s(SP_6) && x <= *r {
            return Some(i as i32);
        }
    }
    None
}

pub fn set_footer_hover(v: Option<i32>) -> bool {
    let want = v.unwrap_or(-1);
    FOOTER_HOVER.swap(want, Ordering::Relaxed) != want
}

struct Chart {
    card: usize,
    rect: [i32; 4],
    xs: Vec<i32>,
}

static CHARTS: Mutex<Vec<Chart>> = Mutex::new(Vec::new());
static CHART_HOVER: AtomicI32 = AtomicI32::new(-1);
static CHART_HOVER_I: AtomicI32 = AtomicI32::new(0);

pub fn chart_hover_at(hwnd: HWND, x: i32, y: i32) -> Option<(i32, i32)> {
    let s = |v: i32| scale(v, dpi_for_window(hwnd));
    let charts = CHARTS.lock().ok()?;
    for c in charts.iter() {
        let [l, t, r, b] = c.rect;
        if y >= t - s(SP_2) && y <= b + s(SP_2) && x >= l - s(CHART_GUTTER) && x <= r {
            let want = x.clamp(l, r);
            let near = c
                .xs
                .iter()
                .enumerate()
                .min_by_key(|(_, sx)| (**sx - want).abs())
                .map(|(i, _)| i as i32)?;
            return Some((c.card as i32, near));
        }
    }
    None
}

pub fn set_chart_hover(v: Option<(i32, i32)>) -> bool {
    let (card, near) = v.unwrap_or((-1, 0));
    let moved = CHART_HOVER.swap(card, Ordering::Relaxed) != card;
    let slid = CHART_HOVER_I.swap(near, Ordering::Relaxed) != near;
    moved || (slid && card >= 0)
}

struct Palette {
    bg: COLORREF,
    text: COLORREF,
    dim: COLORREF,
    good: COLORREF,
    warn: COLORREF,
    low: COLORREF,
    pill_bg: COLORREF,
    track: COLORREF,
    divider: COLORREF,
}

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | ((g as u32) << 8) | ((b as u32) << 16))
}

fn palette() -> Palette {
    if crate::theme::taskbar_is_light() {
        Palette {
            bg: rgb(0xFA, 0xFA, 0xFA),
            text: rgb(0x10, 0x10, 0x10),
            dim: rgb(0x5F, 0x5F, 0x5F),
            good: rgb(0x0E, 0x8A, 0x45),
            warn: rgb(0xB8, 0x6E, 0x00),
            low: rgb(0xC4, 0x2B, 0x1C),
            pill_bg: rgb(0xE3, 0xF5, 0xE8),
            track: rgb(0xE4, 0xE4, 0xE4),
            divider: rgb(0xEA, 0xEA, 0xEA),
        }
    } else {
        Palette {
            bg: rgb(0x1B, 0x1B, 0x1B),
            text: rgb(0xFF, 0xFF, 0xFF),
            dim: rgb(0x8C, 0x8C, 0x8C),
            good: rgb(0x40, 0xD0, 0x6B),
            warn: rgb(0xF5, 0xB5, 0x3D),
            low: rgb(0xFF, 0x5B, 0x4E),
            pill_bg: rgb(0x16, 0x36, 0x22),
            track: rgb(0x2E, 0x2E, 0x2E),
            divider: rgb(0x2B, 0x2B, 0x2B),
        }
    }
}

fn level_color(p: &Palette, pct: Option<f32>, charging: bool) -> COLORREF {
    match pct {
        None => p.dim,
        Some(_) if charging => p.good,
        Some(v) if v <= 10.0 => p.low,
        Some(v) if v <= 25.0 => p.warn,
        Some(_) => p.good,
    }
}

fn scale(v: i32, dpi: i32) -> i32 {
    (v * dpi + 48) / 96
}

fn dpi_for_window(hwnd: HWND) -> i32 {
    unsafe { GetDpiForWindow(hwnd).max(96) as i32 }
}

fn dpi_for_rect(r: RECT) -> u32 {
    unsafe {
        let mon = MonitorFromRect(&r, MONITOR_DEFAULTTONEAREST);
        let (mut x, mut y) = (96u32, 96u32);
        if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut x, &mut y).is_ok() {
            return x.max(96);
        }
        96
    }
}

fn card_count() -> usize {
    state::snapshot().len()
}

pub fn height_for(devices: usize, dpi: u32) -> i32 {
    let s = |v: i32| scale(v, dpi as i32);
    match page() {
        Page::Settings => s(SETTINGS_DIP),
        Page::Battery if devices == 0 => s(PAD) + s(EMPTY_DIP) + s(PAD_BOTTOM),
        Page::Battery => s(PAD) + devices as i32 * s(CARD_DIP) + s(PAD_BOTTOM),
    }
}

pub fn show(hwnd: HWND, anchor: RECT) {
    let dpi = dpi_for_rect(anchor);
    let w = scale(WIDTH_DIP, dpi as i32);
    let h = height_for(card_count(), dpi);
    let gap = scale(SP_3, dpi as i32);

    let mut work = RECT::default();
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut work as *mut RECT as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }

    let mut x = anchor.left + (anchor.right - anchor.left) / 2 - w / 2;
    x = x.clamp(work.left + gap, (work.right - w - gap).max(work.left + gap));
    let above = anchor.top - gap - h;
    let y = if above >= work.top { above } else { anchor.bottom + gap };

    unsafe {
        let pref = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &pref as *const _ as *const _,
            std::mem::size_of_val(&pref) as u32,
        );
        let _ = SetWindowPos(hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
    crate::gip::request_refresh();
}

pub fn resize_to_content(hwnd: HWND) {
    unsafe {
        let mut r = RECT::default();
        if GetWindowRect(hwnd, &mut r).is_err() {
            return;
        }
        let want = height_for(card_count(), dpi_for_window(hwnd) as u32);
        let have = r.bottom - r.top;
        if want == have {
            return;
        }
        // Grow upward so the panel stays anchored to the taskbar edge.
        let _ = SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            r.left,
            r.top - (want - have),
            r.right - r.left,
            want,
            SWP_NOACTIVATE | SWP_NOZORDER,
        );
    }
}

pub fn hide(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
    set_page(Page::Battery);
}

pub fn is_visible(hwnd: HWND) -> bool {
    unsafe { IsWindowVisible(hwnd).as_bool() }
}

pub fn gear_rect(hwnd: HWND) -> RECT {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    let s = |v: i32| scale(v, dpi_for_window(hwnd));
    RECT {
        left: rc.right - s(PAD) - s(SP_6),
        top: s(PAD),
        right: rc.right - s(PAD) + s(SP_1),
        bottom: s(PAD) + s(28),
    }
}

#[derive(Clone, Copy)]
struct Font {
    px: i32,
    weight: i32,
    family: PCWSTR,
}

fn ui(px: i32, weight: i32) -> Font {
    Font { px, weight, family: w!("Segoe UI") }
}

fn glyph(px: i32) -> Font {
    Font { px, weight: FW_NORMAL.0 as i32, family: icon_font() }
}

struct Painter {
    dc: HDC,
    dpi: i32,
    bits: *mut u32,
    bw: i32,
    bh: i32,
}

impl Painter {
    fn s(&self, v: i32) -> i32 {
        scale(v, self.dpi)
    }

    fn sync(&self) {
        if !self.bits.is_null() {
            unsafe {
                let _ = GdiFlush();
            }
        }
    }

    /// Source-over one pixel. `a` is coverage in 0..1.
    #[inline]
    fn blend(&self, x: i32, y: i32, c: COLORREF, a: f32) {
        if self.bits.is_null() || a <= 0.002 || x < 0 || y < 0 || x >= self.bw || y >= self.bh {
            return;
        }
        let a = a.min(1.0);
        // COLORREF is 0x00BBGGRR; a BI_RGB DIB pixel is 0x00RRGGBB.
        let (sr, sg, sb) = (
            (c.0 & 0xFF) as f32,
            ((c.0 >> 8) & 0xFF) as f32,
            ((c.0 >> 16) & 0xFF) as f32,
        );
        unsafe {
            let px = self.bits.add((y * self.bw + x) as usize);
            let d = *px;
            let (dr, dg, db) = (
                ((d >> 16) & 0xFF) as f32,
                ((d >> 8) & 0xFF) as f32,
                (d & 0xFF) as f32,
            );
            let mix = |s: f32, dst: f32| ((dst + (s - dst) * a).round().clamp(0.0, 255.0)) as u32;
            *px = (mix(sr, dr) << 16) | (mix(sg, dg) << 8) | mix(sb, db);
        }
    }

    fn wash(&self, x0: i32, y0: i32, x1: i32, y1: i32, c: COLORREF, a: f32) {
        if self.bits.is_null() {
            return;
        }
        self.sync();
        for y in y0.max(0)..y1.min(self.bh) {
            for x in x0.max(0)..x1.min(self.bw) {
                self.blend(x, y, c, a);
            }
        }
    }

    fn font(&self, f: Font) -> HFONT {
        unsafe {
            CreateFontW(
                -self.s(f.px),
                0,
                0,
                0,
                f.weight,
                0,
                0,
                0,
                0,
                Default::default(),
                Default::default(),
                CLEARTYPE_QUALITY.0 as u32,
                Default::default(),
                f.family,
            )
        }
    }

    fn draw(&self, s: &str, rect: RECT, font: Font, color: COLORREF, flags: DRAW_TEXT_FORMAT) {
        unsafe {
            let f = self.font(font);
            let old = SelectObject(self.dc, f);
            SetTextColor(self.dc, color);
            SetBkMode(self.dc, TRANSPARENT);
            let mut wide: Vec<u16> = s.encode_utf16().collect();
            let mut r = rect;
            DrawTextW(self.dc, &mut wide, &mut r, flags);
            SelectObject(self.dc, old);
            let _ = DeleteObject(f);
        }
    }

    fn text(&self, s: &str, r: RECT, px: i32, w: i32, c: COLORREF, f: DRAW_TEXT_FORMAT) {
        self.draw(s, r, ui(px, w), c, f);
    }

    fn icon(&self, s: &str, r: RECT, px: i32, c: COLORREF, f: DRAW_TEXT_FORMAT) {
        self.draw(s, r, glyph(px), c, f);
    }

    fn width_of(&self, s: &str, font: Font) -> i32 {
        unsafe {
            let f = self.font(font);
            let old = SelectObject(self.dc, f);
            let mut sz = Default::default();
            let wide: Vec<u16> = s.encode_utf16().collect();
            let _ = windows::Win32::Graphics::Gdi::GetTextExtentPoint32W(self.dc, &wide, &mut sz);
            SelectObject(self.dc, old);
            let _ = DeleteObject(f);
            sz.cx
        }
    }

    fn round_fill(&self, x0: i32, y0: i32, x1: i32, y1: i32, r: i32, c: COLORREF) {
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        if self.bits.is_null() {
            unsafe {
                let brush = CreateSolidBrush(c);
                let pen = CreatePen(PS_SOLID, 1, c);
                let ob = SelectObject(self.dc, brush);
                let op = SelectObject(self.dc, pen);
                let _ = RoundRect(self.dc, x0, y0, x1, y1, r * 2, r * 2);
                SelectObject(self.dc, ob);
                SelectObject(self.dc, op);
                let _ = DeleteObject(brush);
                let _ = DeleteObject(pen);
            }
            return;
        }
        self.sync();
        let (fx0, fy0) = (x0 as f32, y0 as f32);
        let (fx1, fy1) = (x1 as f32, y1 as f32);
        let r = (r as f32).min((fx1 - fx0) / 2.0).min((fy1 - fy0) / 2.0).max(0.0);
        let (cx, cy) = ((fx0 + fx1) / 2.0, (fy0 + fy1) / 2.0);
        let (hx, hy) = ((fx1 - fx0) / 2.0 - r, (fy1 - fy0) / 2.0 - r);
        for y in y0.max(0)..y1.min(self.bh) {
            for x in x0.max(0)..x1.min(self.bw) {
                let qx = (x as f32 + 0.5 - cx).abs() - hx;
                let qy = (y as f32 + 0.5 - cy).abs() - hy;
                let outside = qx.max(0.0).hypot(qy.max(0.0));
                let d = outside + qx.max(qy).min(0.0) - r;
                self.blend(x, y, c, (0.5 - d).clamp(0.0, 1.0));
            }
        }
    }

    fn hline(&self, x0: i32, x1: i32, y: i32, c: COLORREF) {
        unsafe {
            let pen = CreatePen(PS_SOLID, 1, c);
            let op = SelectObject(self.dc, pen);
            let _ = MoveToEx(self.dc, x0, y, None);
            let _ = LineTo(self.dc, x1, y);
            SelectObject(self.dc, op);
            let _ = DeleteObject(pen);
        }
    }

    fn curve(&self, pts: &[POINT], faint: &[bool], half: f32, c: COLORREF, dim: f32) {
        if pts.len() < 2 {
            return;
        }
        if self.bits.is_null() {
            unsafe {
                let pen = CreatePen(PS_SOLID, (half * 2.0) as i32, c);
                let op = SelectObject(self.dc, pen);
                let _ = Polyline(self.dc, pts);
                SelectObject(self.dc, op);
                let _ = DeleteObject(pen);
            }
            return;
        }
        self.sync();
        let grow = half.ceil() as i32 + 1;
        let (mut bx0, mut by0, mut bx1, mut by1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for q in pts {
            bx0 = bx0.min(q.x);
            by0 = by0.min(q.y);
            bx1 = bx1.max(q.x);
            by1 = by1.max(q.y);
        }
        let (bx0, by0) = ((bx0 - grow).max(0), (by0 - grow).max(0));
        let (bx1, by1) = ((bx1 + grow).min(self.bw - 1), (by1 + grow).min(self.bh - 1));
        if bx1 < bx0 || by1 < by0 {
            return;
        }
        let (mw, mh) = ((bx1 - bx0 + 1) as usize, (by1 - by0 + 1) as usize);
        let mut mask = vec![0f32; mw * mh];

        for (i, seg) in pts.windows(2).enumerate() {
            let opacity = if faint.get(i).copied().unwrap_or(false) { dim } else { 1.0 };
            let (a, b) = (seg[0], seg[1]);
            let (ax, ay) = (a.x as f32 + 0.5, a.y as f32 + 0.5);
            let (dx, dy) = (b.x as f32 - a.x as f32, b.y as f32 - a.y as f32);
            let len2 = dx * dx + dy * dy;
            let sx0 = (a.x.min(b.x) - grow).max(bx0);
            let sy0 = (a.y.min(b.y) - grow).max(by0);
            let sx1 = (a.x.max(b.x) + grow).min(bx1);
            let sy1 = (a.y.max(b.y) + grow).min(by1);
            for y in sy0..=sy1 {
                for x in sx0..=sx1 {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let t = if len2 > 0.0 {
                        (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let d = (px - (ax + t * dx)).hypot(py - (ay + t * dy));
                    let cov = (half + 0.5 - d).clamp(0.0, 1.0) * opacity;
                    let idx = (y - by0) as usize * mw + (x - bx0) as usize;
                    if cov > mask[idx] {
                        mask[idx] = cov;
                    }
                }
            }
        }
        for j in 0..mh {
            for i in 0..mw {
                self.blend(bx0 + i as i32, by0 + j as i32, c, mask[j * mw + i]);
            }
        }
    }

    /// Vertical fade from the line down to the baseline.
    fn under_curve(&self, pts: &[POINT], bottom: i32, c: COLORREF, peak: f32) {
        if self.bits.is_null() || pts.len() < 2 {
            return;
        }
        self.sync();
        let last = pts[pts.len() - 1].x;
        for seg in pts.windows(2) {
            let (a, b) = (seg[0], seg[1]);
            if b.x <= a.x {
                continue;
            }
            for x in a.x..=(if b.x == last { b.x } else { b.x - 1 }) {
                let t = (x - a.x) as f32 / (b.x - a.x) as f32;
                let top = a.y as f32 + (b.y - a.y) as f32 * t;
                let first = top.ceil() as i32;
                self.blend(x, first - 1, c, peak * (first as f32 - top));
                let depth = (bottom - first).max(1) as f32;
                for y in first..bottom {
                    let k = 1.0 - (y - first) as f32 / depth;
                    self.blend(x, y, c, peak * k * k);
                }
            }
        }
    }
}

fn axis_bounds(lo: f32, hi: f32) -> (f32, f32, f32) {
    let step = match hi - lo {
        d if d <= 10.0 => 5.0,
        d if d <= 40.0 => 10.0,
        _ => 25.0,
    };
    let mut b0 = ((lo / step).floor() * step).max(0.0);
    let mut b1 = ((hi / step).ceil() * step).min(100.0);
    while b1 - b0 < step * 2.0 {
        if b1 < 100.0 {
            b1 = (b1 + step).min(100.0);
        } else if b0 > 0.0 {
            b0 = (b0 - step).max(0.0);
        } else {
            break;
        }
    }
    (b0, b1, step)
}

fn clock_label(at: SystemTime) -> Option<String> {
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::System::SystemInformation::GetLocalTime;
    use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToFileTime};

    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

    let age = SystemTime::now().duration_since(at).ok()?.as_secs();
    unsafe {
        let local = GetLocalTime();
        let mut ft = FILETIME::default();
        SystemTimeToFileTime(&local, &mut ft).ok()?;
        let ticks = (((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64)
            .checked_sub(age.checked_mul(10_000_000)?)?;
        let back = FILETIME {
            dwLowDateTime: ticks as u32,
            dwHighDateTime: (ticks >> 32) as u32,
        };
        let mut out = SYSTEMTIME::default();
        FileTimeToSystemTime(&back, &mut out).ok()?;
        let same_day =
            out.wYear == local.wYear && out.wMonth == local.wMonth && out.wDay == local.wDay;
        let clock = format!("{:02}:{:02}", out.wHour, out.wMinute);
        Some(if same_day {
            clock
        } else if age < 7 * 24 * 60 * 60 {
            match DAYS.get(out.wDayOfWeek as usize) {
                Some(day) => format!("{day} {clock}"),
                None => clock,
            }
        } else {
            format!("{:02}/{:02} {clock}", out.wDay, out.wMonth)
        })
    }
}

unsafe fn dib(dc: HDC, w: i32, h: i32) -> (HBITMAP, *mut u32) {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    match CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, None, 0) {
        Ok(b) if !bits.is_null() => (b, bits as *mut u32),
        _ => (CreateCompatibleBitmap(dc, w, h), std::ptr::null_mut()),
    }
}

pub fn on_paint(hwnd: HWND) -> LRESULT {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);

        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let (cw, ch) = (rc.right - rc.left, rc.bottom - rc.top);

        let mem = CreateCompatibleDC(hdc);
        let (bmp, bits) = dib(mem, cw, ch);
        let oldbmp = SelectObject(mem, bmp);

        let p = palette();
        let g = Painter {
            dc: mem,
            dpi: dpi_for_window(hwnd),
            bits,
            bw: cw,
            bh: ch,
        };

        let bg: HBRUSH = CreateSolidBrush(p.bg);
        FillRect(mem, &rc, bg);
        let _ = DeleteObject(bg);

        match page() {
            Page::Settings => draw_settings(&g, &p, cw),
            Page::Battery => {
                if let Ok(mut f) = FOOTERS.lock() {
                    f.clear();
                }
                if let Ok(mut c) = CHARTS.lock() {
                    c.clear();
                }
                let devices = state::snapshot();
                let mut y = g.s(PAD);
                if devices.is_empty() {
                    draw_empty(&g, &p, cw, y);
                }
                for (i, d) in devices.iter().enumerate() {
                    if i > 0 {
                        g.hline(g.s(PAD), cw - g.s(PAD), y - g.s(10), p.divider);
                    }
                    draw_card(&g, &p, cw, y, d, i);
                    y += g.s(CARD_DIP);
                }
            }
        }

        let _ = BitBlt(hdc, 0, 0, cw, ch, mem, 0, 0, SRCCOPY);
        SelectObject(mem, oldbmp);
        let _ = DeleteObject(bmp);
        let _ = DeleteDC(mem);
        let _ = EndPaint(hwnd, &ps);
        LRESULT(0)
    }
}

fn rows_top(s: impl Fn(i32) -> i32) -> i32 {
    s(PAD) + s(HEADER_DIP)
}

pub fn settings_row_at(hwnd: HWND, x: i32, y: i32) -> Option<usize> {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    let s = |v: i32| scale(v, dpi_for_window(hwnd));
    if x < s(PAD) - s(SP_2) || x > rc.right - s(PAD) + s(SP_2) {
        return None;
    }
    let top = rows_top(s);
    if y < top {
        return None;
    }
    let idx = (y - top) / s(ROW_DIP);
    (idx >= 0 && (idx as usize) < ROW_COUNT).then_some(idx as usize)
}

fn draw_settings(g: &Painter, p: &Palette, cw: i32) {
    let pad = g.s(PAD);
    let right = cw - pad;

    g.text(
        "Settings",
        RECT { left: pad, top: pad, right: right - g.s(SP_6), bottom: pad + g.s(28) },
        TX_TITLE,
        FW_SEMIBOLD.0 as i32,
        p.text,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );

    g.icon(
        GLYPH_BACK,
        RECT {
            left: cw - pad - g.s(SP_6),
            top: pad,
            right: cw - pad + g.s(SP_1),
            bottom: pad + g.s(28),
        },
        GL_MEDIUM,
        p.dim,
        DT_CENTER | DT_SINGLELINE | DT_VCENTER,
    );

    let hover = HOVER.load(Ordering::Relaxed);
    let top = rows_top(|v| g.s(v));
    for i in 0..ROW_COUNT {
        let ry = top + i as i32 * g.s(ROW_DIP);
        let rb = ry + g.s(ROW_DIP) - g.s(SP_1);
        if hover == i as i32 {
            g.round_fill(pad - g.s(SP_2), ry, right + g.s(SP_2), rb, g.s(SP_1), p.track);
        }
        let label = match i {
            ROW_REFRESH => "Refresh now",
            ROW_AUTOSTART => "Start with Windows",
            _ => "Quit",
        };
        g.text(
            label,
            RECT { left: pad, top: ry, right: right - g.s(SP_6), bottom: rb },
            TX_BODY,
            FW_NORMAL.0 as i32,
            p.text,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );
        if i == ROW_AUTOSTART && crate::theme::autostart_enabled() {
            g.icon(
                GLYPH_CHECK,
                RECT { left: right - g.s(GL_LARGE), top: ry, right, bottom: rb },
                GL_SMALL,
                p.good,
                DT_LEFT | DT_SINGLELINE | DT_VCENTER,
            );
        }
    }
}

fn draw_empty(g: &Painter, p: &Palette, cw: i32, y: i32) {
    let pad = g.s(PAD);
    g.icon(
        GLYPH_GEAR,
        RECT { left: cw - pad - g.s(SP_6), top: pad, right: cw - pad + g.s(SP_1), bottom: pad + g.s(28) },
        GL_MEDIUM,
        p.dim,
        DT_CENTER | DT_SINGLELINE | DT_VCENTER,
    );
    g.icon(
        GLYPH_GAMEPAD,
        RECT { left: pad, top: y, right: pad + g.s(28), bottom: y + g.s(28) },
        GL_LARGE,
        p.dim,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
    );
    g.text(
        "No controller connected",
        RECT { left: pad + g.s(36), top: y, right: cw - pad, bottom: y + g.s(28) },
        TX_TITLE,
        FW_SEMIBOLD.0 as i32,
        p.text,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );
    g.text(
        "The pad leaves USB entirely when it is powered off. Press the logo button to wake it.",
        RECT { left: pad, top: y + g.s(40), right: cw - pad, bottom: y + g.s(112) },
        TX_BODY,
        FW_NORMAL.0 as i32,
        p.dim,
        DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
    );
}

fn draw_card(g: &Painter, p: &Palette, cw: i32, top: i32, d: &Device, index: usize) {
    let first = index == 0;
    let pad = g.s(PAD);
    let right = cw - pad;
    let mut y = top;

    // ---- header -----------------------------------------------------------
    g.icon(
        GLYPH_GAMEPAD,
        RECT { left: pad, top: y, right: pad + g.s(28), bottom: y + g.s(HEAD_H) },
        GL_LARGE,
        p.text,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
    );
    let name_x = pad + g.s(36);
    g.text(
        d.name,
        RECT { left: name_x, top: y, right: right - g.s(SP_6), bottom: y + g.s(SP_5) },
        TX_TITLE,
        FW_SEMIBOLD.0 as i32,
        p.text,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );
    let subtitle = if !d.connected {
        "Disconnected".to_string()
    } else {
        d.link.label().unwrap_or("Connected").to_string()
    };
    g.text(
        &subtitle,
        RECT { left: name_x, top: y + g.s(SP_5), right: right - g.s(SP_6), bottom: y + g.s(HEAD_H) },
        TX_BODY,
        FW_NORMAL.0 as i32,
        p.dim,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );
    if first {
        g.icon(
            GLYPH_GEAR,
            RECT { left: right - g.s(SP_6), top: y, right: right + g.s(SP_1), bottom: y + g.s(28) },
            GL_MEDIUM,
            p.dim,
            DT_CENTER | DT_SINGLELINE | DT_VCENTER,
        );
    }
    y += g.s(HEAD_H) + g.s(SP_4);

    // ---- level ------------------------------------------------------------
    let pct = d.percent();
    let charging = d.is_charging() && d.connected;
    let col = level_color(p, pct, charging);
    let big = match pct {
        Some(v) => format!("{}%", v.round() as i32),
        None => "\u{2014}".to_string(),
    };
    g.text(
        &big,
        RECT { left: pad, top: y, right, bottom: y + g.s(LEVEL_H) },
        TX_DISPLAY,
        FW_SEMIBOLD.0 as i32,
        col,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );

    if charging {
        let label = "Charging";
        let tw = g.width_of(label, ui(TX_BODY, FW_SEMIBOLD.0 as i32));
        let iw = g.width_of(GLYPH_BOLT, glyph(GL_SMALL));
        // Inset, glyph, gap, text, inset.
        let pill_w = g.s(SP_2) + iw + g.s(SP_1) + tw + g.s(SP_2);
        let pill_h = g.s(SP_6);
        let px0 = pad + g.width_of(&big, ui(TX_DISPLAY, FW_SEMIBOLD.0 as i32)) + g.s(SP_4);
        let py0 = y + (g.s(LEVEL_H) - pill_h) / 2;
        g.round_fill(px0, py0, px0 + pill_w, py0 + pill_h, pill_h / 2, p.pill_bg);
        g.icon(
            GLYPH_BOLT,
            RECT { left: px0 + g.s(SP_2), top: py0, right: px0 + g.s(SP_2) + iw, bottom: py0 + pill_h },
            GL_SMALL,
            p.good,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        );
        g.text(
            label,
            RECT {
                left: px0 + g.s(SP_2) + iw + g.s(SP_1),
                top: py0,
                right: px0 + pill_w,
                bottom: py0 + pill_h,
            },
            TX_BODY,
            FW_SEMIBOLD.0 as i32,
            p.good,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );
    }
    y += g.s(LEVEL_H) + g.s(SP_3);

    // ---- bar --------------------------------------------------------------
    let bar_h = g.s(SP_2);
    g.round_fill(pad, y, right, y + bar_h, bar_h / 2, p.track);
    if let Some(v) = pct {
        let filled = ((right - pad) as f32 * (v / 100.0).clamp(0.0, 1.0)) as i32;
        g.round_fill(pad, y, pad + filled.max(bar_h), y + bar_h, bar_h / 2, col);
    }
    y += bar_h + g.s(SP_4);

    // ---- history ----------------------------------------------------------
    y = draw_history(g, p, cw, y, d, index, col) + g.s(SP_2);

    // ---- footer -----------------------------------------------------------
    let bot = y + g.s(FOOT_H);
    if let Ok(mut f) = FOOTERS.lock() {
        f.push([pad, y, right, bot]);
    }

    let dot = g.s(SP_2);
    let dot_y = y + (g.s(FOOT_H) - dot) / 2;
    let fresh = match d.battery.map(|s| s.age().as_secs()) {
        Some(a) if a <= 120 => p.good,
        Some(a) if a <= 1800 => p.warn,
        _ => p.low,
    };
    g.round_fill(pad, dot_y, pad + dot, dot_y + dot, dot / 2, fresh);

    if FOOTER_HOVER.load(Ordering::Relaxed) == index as i32 {
        g.text(
            "Reads only while this panel is open",
            RECT { left: pad + g.s(64), top: y, right: right - g.s(SP_6), bottom: bot },
            TX_META,
            FW_NORMAL.0 as i32,
            p.dim,
            DT_RIGHT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );
    } else {
        let age = match d.battery {
            Some(s) => format!("Updated {}", state::describe_age(s.age())),
            None if crate::gip::transport_ever_worked() => "No reading yet".to_string(),
            None => "Battery query unavailable".to_string(),
        };
        g.text(
            &age,
            RECT { left: pad + dot + g.s(SP_2), top: y, right: right - g.s(SP_6), bottom: bot },
            TX_META,
            FW_NORMAL.0 as i32,
            p.dim,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );
    }
    g.icon(
        GLYPH_INFO,
        RECT { left: right - g.s(GL_MEDIUM), top: y, right, bottom: bot },
        GL_SMALL,
        p.dim,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
    );
}

fn rate_label(d: &Device) -> String {
    let Some(r) = d.rate_per_min().filter(|r| r.abs() >= 0.05) else {
        return String::new();
    };
    let mut out = format!("{r:+.1}%/min");
    match d.minutes_to_full() {
        Some(m) if m >= 60 => out.push_str(&format!(" \u{00B7} full in ~{}h {:02}m", m / 60, m % 60)),
        Some(m) => out.push_str(&format!(" \u{00B7} full in ~{m} min")),
        None => {}
    }
    out
}

fn retained_history(d: &Device, now: SystemTime) -> Vec<(f32, f32, SystemTime)> {
    d.history
        .iter()
        .filter_map(|s| {
            let age = now.duration_since(s.at).ok()?.as_secs_f32();
            Some((age, crate::protocol::percent(s.raw), s.at))
        })
        .collect()
}

fn draw_history(
    g: &Painter,
    p: &Palette,
    cw: i32,
    top: i32,
    d: &Device,
    index: usize,
    col: COLORREF,
) -> i32 {
    let pad = g.s(PAD);
    let right = cw - pad;
    let mut y = top;

    g.hline(pad, right, y, p.divider);
    y += g.s(SP_4);

    let plot_t = y + g.s(SP_4) + g.s(SP_2);
    let plot_b = plot_t + g.s(CHART_H);
    let plot_l = pad + g.s(CHART_GUTTER);
    let plot_r = right;
    let below = plot_b + g.s(AXIS_H);
    let head = RECT { left: plot_l, top: y, right, bottom: y + g.s(SP_4) };

    let now = SystemTime::now();
    let recent = retained_history(d, now);

    if recent.len() < 2 || plot_r - plot_l < g.s(SP_6 * 2) {
        g.text(
            "History appears after two readings.",
            RECT { left: pad, top: plot_t, right, bottom: below },
            TX_META,
            FW_NORMAL.0 as i32,
            p.dim,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );
        return below;
    }

    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for (_, v, _) in &recent {
        lo = lo.min(*v);
        hi = hi.max(*v);
    }
    let (b0, b1, step) = axis_bounds(lo, hi);

    let oldest = recent[0].0;
    let newest = recent[recent.len() - 1].0;
    let span = (oldest - newest).max(1.0);
    let (pw, ph) = ((plot_r - plot_l) as f32, (plot_b - plot_t) as f32);
    let screen: Vec<POINT> = recent
        .iter()
        .map(|(age, v, _)| POINT {
            x: plot_l + (((oldest - age) / span) * pw) as i32,
            y: plot_b - (((v - b0) / (b1 - b0)).clamp(0.0, 1.0) * ph) as i32,
        })
        .collect();

    let gap_limit = (span * 0.1).max(900.0);
    let faint: Vec<bool> = recent.windows(2).map(|w| w[0].0 - w[1].0 > gap_limit).collect();

    if let Ok(mut c) = CHARTS.lock() {
        c.push(Chart {
            card: index,
            rect: [plot_l, plot_t, plot_r, plot_b],
            xs: screen.iter().map(|q| q.x).collect(),
        });
    }

    let hovered = (CHART_HOVER.load(Ordering::Relaxed) == index as i32)
        .then(|| CHART_HOVER_I.load(Ordering::Relaxed) as usize)
        .filter(|i| *i < screen.len());

    // ---- headline ---------------------------------------------------------
    let (headline, head_col) = match hovered {
        Some(i) => {
            let (_, v, at) = recent[i];
            let mut t = format!("{}%", v.round() as i32);
            if let Some(c) = clock_label(at) {
                t.push_str(&format!(" \u{00B7} {c}"));
            }
            (t, p.text)
        }
        None => (rate_label(d), p.dim),
    };
    if !headline.is_empty() {
        g.text(
            &headline,
            head,
            TX_META,
            FW_NORMAL.0 as i32,
            head_col,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );
    }

    // ---- grid and scale ---------------------------------------------------
    let mut v = b0;
    while v <= b1 + 0.01 {
        let gy = plot_b - (((v - b0) / (b1 - b0)) * ph) as i32;
        g.hline(plot_l, plot_r, gy, p.divider);
        v += step;
    }
    for (val, gy) in [(b1, plot_t), (b0, plot_b)] {
        g.text(
            &format!("{}", val as i32),
            RECT { left: pad, top: gy - g.s(SP_2), right: plot_l - g.s(SP_2), bottom: gy + g.s(SP_2) },
            TX_META,
            FW_NORMAL.0 as i32,
            p.dim,
            DT_RIGHT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );
    }

    if let Some(i) = hovered {
        g.wash(screen[i].x, plot_t, screen[i].x + 1, plot_b, p.dim, 0.5);
    }

    // ---- the line ---------------------------------------------------------
    g.under_curve(&screen, plot_b, col, 0.20);
    g.curve(&screen, &faint, (g.s(2) as f32 / 2.0).max(1.0), col, 0.3);

    if let Some(i) = hovered {
        let (q, r) = (screen[i], g.s(SP_1));
        let ring = r + g.s(SP_1) / 2;
        g.round_fill(q.x - ring, q.y - ring, q.x + ring, q.y + ring, ring, p.bg);
        g.round_fill(q.x - r, q.y - r, q.x + r, q.y + r, r, col);
    }

    // ---- time axis --------------------------------------------------------
    let axis = |t: &str, l: i32, r: i32, f: DRAW_TEXT_FORMAT| {
        g.text(
            t,
            RECT { left: l, top: plot_b + g.s(SP_1), right: r, bottom: below },
            TX_META,
            FW_NORMAL.0 as i32,
            p.dim,
            f | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        )
    };
    if let Some(t) = clock_label(recent[0].2) {
        axis(&t, plot_l, plot_l + g.s(88), DT_LEFT);
    }
    if let Some(t) = clock_label(recent[recent.len() - 1].2) {
        axis(&t, plot_r - g.s(88), plot_r, DT_RIGHT);
    }

    below
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn device(raw: Option<u8>, age: u64, charging: bool, history: &[(u8, u64)]) -> Device {
        let now = SystemTime::now();
        Device {
            key: 1,
            vid: 0x1532,
            pid: 0x0A3F,
            name: "Wolverine V3 Pro",
            link: state::Link::Dongle,
            connected: true,
            readable: true,
            battery: raw.map(|r| state::Sample { raw: r, at: now - Duration::from_secs(age) }),
            charging: Some((charging, now)),
            history: history
                .iter()
                .map(|(r, s)| state::Sample { raw: *r, at: now - Duration::from_secs(*s) })
                .collect(),
        }
    }

    #[test]
    fn chart_keeps_retained_readings_older_than_a_day() {
        let d = device(
            Some(104),
            60,
            false,
            &[(175, 2 * 24 * 60 * 60), (104, 60)],
        );

        let points = retained_history(&d, SystemTime::now());
        assert_eq!(
            points.len(),
            2,
            "the older persisted reading must remain plottable"
        );
    }

    /// The scale factors the Windows display settings slider offers.
    const SLIDER_FACTORS: [i32; 7] = [96, 120, 144, 168, 192, 216, 240];
    /// Plus the custom values Windows accepts if they are typed in.
    const CUSTOM_FACTORS: [i32; 3] = [108, 132, 156];

    const RAMP: [i32; 6] = [SP_1, SP_2, SP_3, SP_4, SP_5, SP_6];

    #[test]
    fn every_dimension_is_on_the_four_dip_grid() {
        for v in [
            SP_1, SP_2, SP_3, SP_4, SP_5, SP_6, PAD, PAD_BOTTOM, CARD_DIP, EMPTY_DIP,
            CHART_GUTTER, CHART_H, AXIS_H, HEAD_H, LEVEL_H, FOOT_H, ROW_DIP, HEADER_DIP,
            SETTINGS_DIP, GL_SMALL, GL_MEDIUM, GL_LARGE,
        ] {
            assert_eq!(v % 4, 0, "{v} DIP is off the 4 DIP grid");
        }
    }

    #[test]
    fn the_ramp_needs_no_rounding_at_any_slider_factor() {
        for dpi in SLIDER_FACTORS {
            for step in RAMP {
                assert_eq!(
                    step * dpi % 96,
                    0,
                    "{step} DIP does not land on a whole pixel at dpi {dpi}"
                );
                assert_eq!(scale(step, dpi), step * dpi / 96);
            }
        }
    }

    #[test]
    fn the_ramp_stays_ordered_at_custom_factors() {
        for dpi in SLIDER_FACTORS.iter().chain(CUSTOM_FACTORS.iter()) {
            for pair in RAMP.windows(2) {
                let (a, b) = (scale(pair[0], *dpi), scale(pair[1], *dpi));
                assert!(a < b, "{} and {} both scale to {a}px at dpi {dpi}", pair[0], pair[1]);
            }
            for step in RAMP {
                let exact = step * dpi;
                assert!(
                    (scale(step, *dpi) * 96 - exact).abs() <= 48,
                    "{step} DIP is more than half a pixel out at dpi {dpi}"
                );
            }
        }
    }

    #[test]
    fn the_card_holds_its_layout_at_every_scale_factor() {
        for dpi in SLIDER_FACTORS.iter().chain(CUSTOM_FACTORS.iter()).copied() {
            let s = |v: i32| scale(v, dpi);
            let content = s(HEAD_H)
                + s(SP_4)
                + s(LEVEL_H)
                + s(SP_3)
                + s(SP_2)
                + s(SP_4)
                + s(SP_4)
                + s(SP_4)
                + s(SP_2)
                + s(CHART_H)
                + s(AXIS_H)
                + s(SP_2)
                + s(FOOT_H);
            assert!(
                content <= s(CARD_DIP) - s(SP_2),
                "at dpi {dpi} the card needs {content}px but CARD_DIP gives {}px",
                s(CARD_DIP)
            );
            let head_bottom = s(SP_4) + s(SP_4);
            let gutter_top = s(SP_4) + s(SP_4) + s(SP_2) - s(SP_2);
            assert!(
                head_bottom <= gutter_top,
                "at dpi {dpi} the chart headline overlaps the {gutter_top}px gutter label"
            );
        }
    }

    fn render(w: i32, h: i32, dpi: i32, f: impl FnOnce(&Painter, &Palette)) -> Vec<u8> {
        unsafe {
            let dc = CreateCompatibleDC(None);
            let (bmp, bits) = dib(dc, w, h);
            assert!(!bits.is_null(), "the sheet needs the same surface the panel uses");
            let old = SelectObject(dc, bmp);

            let p = palette();
            let brush = CreateSolidBrush(p.bg);
            FillRect(dc, &RECT { left: 0, top: 0, right: w, bottom: h }, brush);
            let _ = DeleteObject(brush);

            f(&Painter { dc, dpi, bits, bw: w, bh: h }, &p);

            let _ = GdiFlush();
            let out = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize).to_vec();
            SelectObject(dc, old);
            let _ = DeleteObject(bmp);
            let _ = DeleteDC(dc);
            out
        }
    }

    #[test]
    fn dump_settings_sheet() {
        let dpi = 144;
        let (w, h) = (scale(WIDTH_DIP, dpi), scale(SETTINGS_DIP, dpi));
        let pixels = render(w, h, dpi, |g, p| {
            HOVER.store(ROW_AUTOSTART as i32, Ordering::Relaxed);
            draw_settings(g, p, w);
            HOVER.store(-1, Ordering::Relaxed);
        });
        write_bmp("target/settings.bmp", &pixels, w, h);
    }

    #[test]
    fn dump_card_sheet() {
        let dpi = 144;
        let s = |v: i32| v * dpi / 96;
        let mut disconnected = device(
            Some(104),
            13 * 60,
            false,
            &[
                (226, 30 * 24 * 60 * 60),
                (175, 2 * 24 * 60 * 60),
                (104, 13 * 60),
            ],
        );
        disconnected.connected = false;
        disconnected.readable = false;
        let cards = [
            device(Some(255), 2, false, &[(250, 5400), (252, 3600), (254, 1800), (255, 30)]),
            device(Some(150), 60, true, &[(120, 900), (130, 600), (140, 300), (150, 60)]),
            // Low, with a day-long record full of holes.
            device(
                Some(20),
                30,
                false,
                &[(90, 82000), (85, 80000), (60, 20000), (45, 9000), (30, 3000), (20, 60)],
            ),
            // Persisted history still renders while disconnected, including
            // numeric dates for readings too old for a weekday label.
            disconnected,
            // Never read.
            device(None, 0, false, &[]),
        ];

        let w = s(WIDTH_DIP);
        let h = s(PAD) + cards.len() as i32 * s(CARD_DIP);

        let pixels = render(w, h, dpi, |g, p| {
            if let Ok(mut f) = FOOTERS.lock() {
                f.clear();
            }
            if let Ok(mut c) = CHARTS.lock() {
                c.clear();
            }
            CHART_HOVER.store(0, Ordering::Relaxed);
            CHART_HOVER_I.store(2, Ordering::Relaxed);
            let mut y = s(PAD);
            for (i, d) in cards.iter().enumerate() {
                FOOTER_HOVER.store(if i == 1 { 1 } else { -1 }, Ordering::Relaxed);
                if i > 0 {
                    g.hline(s(PAD), w - s(PAD), y - s(SP_2), p.divider);
                }
                draw_card(g, p, w, y, d, i);
                y += s(CARD_DIP);
            }
            FOOTER_HOVER.store(-1, Ordering::Relaxed);
            CHART_HOVER.store(-1, Ordering::Relaxed);
        });

        let footers = FOOTERS.lock().unwrap().clone();
        assert_eq!(footers.len(), cards.len());
        for (i, [_, top, _, bottom]) in footers.iter().enumerate() {
            let card_bottom = s(PAD) + (i as i32 + 1) * s(CARD_DIP);
            assert!(
                *bottom <= card_bottom - s(SP_2),
                "card {i} footer runs {top}..{bottom} but the card ends at \
                 {card_bottom}; CARD_DIP is too small by {}px",
                bottom + s(SP_2) - card_bottom
            );
        }

        write_bmp("target/cards.bmp", &pixels, w, h);
    }

    /// The surface is top-down and a .bmp with a positive height is bottom-up,
    /// so the rows go back in reverse.
    fn write_bmp(rel: &str, pixels: &[u8], w: i32, h: i32) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
        let bytes = (w * h * 4) as u32;
        let mut f = Vec::with_capacity(54 + bytes as usize);
        f.extend_from_slice(b"BM");
        f.extend_from_slice(&(54 + bytes).to_le_bytes());
        f.extend_from_slice(&0u32.to_le_bytes());
        f.extend_from_slice(&54u32.to_le_bytes());
        f.extend_from_slice(&40u32.to_le_bytes());
        f.extend_from_slice(&w.to_le_bytes());
        f.extend_from_slice(&h.to_le_bytes());
        f.extend_from_slice(&1u16.to_le_bytes());
        f.extend_from_slice(&32u16.to_le_bytes());
        f.extend_from_slice(&0u32.to_le_bytes());
        f.extend_from_slice(&bytes.to_le_bytes());
        for _ in 0..4 {
            f.extend_from_slice(&0u32.to_le_bytes());
        }
        for row in pixels.chunks_exact((w * 4) as usize).rev() {
            f.extend_from_slice(row);
        }
        std::fs::write(&path, f).unwrap();
        assert!(path.exists());
    }
}
