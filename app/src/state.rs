
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

pub const WM_STATE_CHANGED: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 1;

static UI_HWND: AtomicIsize = AtomicIsize::new(0);
static DEVICES: Mutex<Vec<Device>> = Mutex::new(Vec::new());

const MAX_HISTORY: usize = 400;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    pub raw: u8,
    pub at: SystemTime,
}

impl Sample {
    pub fn age(&self) -> Duration {
        self.at.elapsed().unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    Wired,
    Dongle,
    Unknown,
}

impl Link {
    pub fn label(self) -> Option<&'static str> {
        match self {
            Link::Wired => Some("USB cable"),
            Link::Dongle => Some("HyperSpeed dongle"),
            Link::Unknown => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Device {
    pub key: u64,
    pub vid: u16,
    pub pid: u16,
    pub name: &'static str,
    pub link: Link,
    /// The pad detaches from USB entirely when powered off.
    pub connected: bool,
    /// False while Windows.Gaming.Input has the input sink suspended, which is
    /// whenever this process is not foreground.
    pub readable: bool,
    pub battery: Option<Sample>,
    pub charging: Option<(bool, SystemTime)>,
    pub history: Vec<Sample>,
}

impl Device {
    pub fn percent(&self) -> Option<f32> {
        self.battery.map(|s| crate::protocol::percent(s.raw))
    }

    pub fn is_charging(&self) -> bool {
        self.charging.map(|(c, _)| c).unwrap_or(false)
    }

    /// Charge rate in percent per minute, least-squares over the last 20
    /// minutes. `None` unless the samples span at least a minute.
    pub fn rate_per_min(&self) -> Option<f32> {
        let now = SystemTime::now();
        let pts: Vec<(f64, f64)> = self
            .history
            .iter()
            .filter_map(|s| {
                let age = now.duration_since(s.at).ok()?.as_secs_f64();
                (age <= 20.0 * 60.0)
                    .then_some((-age / 60.0, crate::protocol::percent(s.raw) as f64))
            })
            .collect();
        if pts.len() < 3 {
            return None;
        }
        let oldest = pts.first()?.0;
        let newest = pts.last()?.0;
        if (newest - oldest).abs() < 1.0 {
            return None; // under a minute of data
        }
        let n = pts.len() as f64;
        let sx: f64 = pts.iter().map(|p| p.0).sum();
        let sy: f64 = pts.iter().map(|p| p.1).sum();
        let sxx: f64 = pts.iter().map(|p| p.0 * p.0).sum();
        let sxy: f64 = pts.iter().map(|p| p.0 * p.1).sum();
        let denom = n * sxx - sx * sx;
        if denom.abs() < 1e-9 {
            return None;
        }
        Some(((n * sxy - sx * sy) / denom) as f32)
    }

    /// Minutes to full, rounded to 5. Suppressed above 90%, where a lithium
    /// cell switches to constant-voltage and stops tracking the observed slope.
    pub fn minutes_to_full(&self) -> Option<u32> {
        if !self.is_charging() {
            return None;
        }
        let pct = self.percent()?;
        if pct > 90.0 {
            return None;
        }
        let rate = self.rate_per_min()?;
        if rate <= 0.05 {
            return None;
        }
        let mins = ((100.0 - pct) / rate).round();
        (mins.is_finite() && mins > 0.0 && mins < 24.0 * 60.0)
            .then(|| ((mins / 5.0).round() as u32 * 5).max(5))
    }
}

pub const KNOWN: &[(u16, u16, &str)] = &[(0x1532, 0x0A3F, "Wolverine V3 Pro")];

pub fn model_name(vid: u16, pid: u16) -> &'static str {
    KNOWN
        .iter()
        .find(|(v, p, _)| *v == vid && *p == pid)
        .map(|(_, _, n)| *n)
        .unwrap_or("Controller")
}

pub fn set_ui_window(hwnd: windows::Win32::Foundation::HWND) {
    UI_HWND.store(hwnd.0 as isize, Ordering::SeqCst);
}

/// Safe from any thread; PostMessage does not block.
pub fn notify_ui() {
    let h = UI_HWND.load(Ordering::SeqCst);
    if h == 0 {
        return;
    }
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
            windows::Win32::Foundation::HWND(h as *mut _),
            WM_STATE_CHANGED,
            windows::Win32::Foundation::WPARAM(0),
            windows::Win32::Foundation::LPARAM(0),
        );
    }
}

pub fn snapshot() -> Vec<Device> {
    DEVICES.lock().map(|d| d.clone()).unwrap_or_default()
}

fn with<R>(key: u64, f: impl FnOnce(&mut Device) -> R) -> Option<R> {
    let mut guard = DEVICES.lock().ok()?;
    guard.iter_mut().find(|d| d.key == key).map(f)
}

/// Reuses the existing row for the same model rather than adding a second one.
pub fn upsert(key: u64, vid: u16, pid: u16, link: Link) {
    if let Ok(mut guard) = DEVICES.lock() {
        if let Some(d) = guard.iter_mut().find(|d| d.vid == vid && d.pid == pid) {
            d.key = key;
            d.connected = true;
            d.link = link;
        } else {
            let prior = crate::store::last_known(vid, pid);
            guard.push(Device {
                key,
                vid,
                pid,
                name: model_name(vid, pid),
                link,
                connected: true,
                readable: false,
                battery: prior.as_ref().map(|e| Sample { raw: e.raw, at: e.at }),
                charging: prior.as_ref().map(|e| (e.charging, e.at)),
                history: prior.map(|e| e.history).unwrap_or_default(),
            });
        }
    }
    notify_ui();
}

/// Saved readings become disconnected cards, so the panel shows the last value
/// before the pad connects. Key 0 is never handed out, so nothing touches these
/// rows until `upsert` claims one by model.
pub fn restore() {
    if let Ok(mut guard) = DEVICES.lock() {
        for (vid, pid, name) in KNOWN {
            let Some(e) = crate::store::last_known(*vid, *pid) else { continue };
            guard.push(Device {
                key: 0,
                vid: *vid,
                pid: *pid,
                name,
                link: Link::Unknown,
                connected: false,
                readable: false,
                battery: Some(Sample { raw: e.raw, at: e.at }),
                charging: Some((e.charging, e.at)),
                history: e.history,
            });
        }
    }
}

pub fn set_connected(key: u64, connected: bool) {
    with(key, |d| {
        d.connected = connected;
        if !connected {
            d.readable = false;
            d.link = Link::Unknown;
        }
    });
    notify_ui();
}

pub fn set_link(key: u64, link: Link) {
    with(key, |d| {
        if d.connected {
            d.link = link;
        }
    });
    notify_ui();
}

pub fn set_readable(key: u64, readable: bool) {
    with(key, |d| d.readable = readable);
    notify_ui();
}

pub fn record_battery(key: u64, raw: u8) {
    let appended = with(key, |d| {
        let now = SystemTime::now();
        let sample = Sample { raw, at: now };
        d.battery = Some(sample);
        // Only extend history when the value moved or a minute passed.
        let keep = match d.history.last() {
            None => true,
            Some(last) => {
                last.raw != raw
                    || now
                        .duration_since(last.at)
                        .map(|g| g.as_secs() >= 60)
                        .unwrap_or(true)
            }
        };
        if keep {
            d.history.push(sample);
            let excess = d.history.len().saturating_sub(MAX_HISTORY);
            d.history.drain(..excess);
        }
        keep
    });
    if appended.unwrap_or(false) {
        persist();
    }
    notify_ui();
}

pub fn record_charging(key: u64, charging: bool) {
    with(key, |d| d.charging = Some((charging, SystemTime::now())));
    notify_ui();
}

pub fn persist() {
    let entries: Vec<crate::store::Entry> = snapshot()
        .iter()
        .filter_map(|d| {
            d.battery.map(|s| crate::store::Entry {
                vid: d.vid,
                pid: d.pid,
                raw: s.raw,
                charging: d.is_charging(),
                at: s.at,
                history: d.history.clone(),
            })
        })
        .collect();
    if !entries.is_empty() {
        crate::store::save(&entries);
    }
}

pub fn describe_age(age: Duration) -> String {
    let s = age.as_secs();
    match s {
        0..=5 => "just now".into(),
        6..=90 => format!("{s}s ago"),
        91..=5400 => format!("{}m ago", (s + 30) / 60),
        _ => format!("{}h ago", (s + 1800) / 3600),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev_with(history: Vec<(u8, u64)>, charging: bool) -> Device {
        let now = SystemTime::now();
        let hist: Vec<Sample> = history
            .iter()
            .map(|(raw, secs_ago)| Sample {
                raw: *raw,
                at: now - Duration::from_secs(*secs_ago),
            })
            .collect();
        Device {
            key: 1,
            vid: 0x1532,
            pid: 0x0A3F,
            name: "test",
            link: Link::Wired,
            connected: true,
            readable: true,
            battery: hist.last().copied(),
            charging: Some((charging, now)),
            history: hist,
        }
    }

    #[test]
    fn rate_needs_more_than_a_moment_of_data() {
        let d = dev_with(vec![(100, 10), (101, 5), (102, 0)], true);
        assert_eq!(d.rate_per_min(), None);
    }

    #[test]
    fn rate_is_positive_while_charging() {
        // 128 -> 153 raw over ten minutes is roughly +1%/min.
        let d = dev_with(vec![(128, 600), (140, 300), (153, 0)], true);
        let r = d.rate_per_min().expect("enough data");
        assert!(r > 0.5 && r < 1.5, "rate was {r}");
    }

    #[test]
    fn no_time_to_full_when_not_charging() {
        let d = dev_with(vec![(128, 600), (140, 300), (153, 0)], false);
        assert_eq!(d.minutes_to_full(), None);
    }

    #[test]
    fn time_to_full_is_suppressed_near_the_top() {
        let d = dev_with(vec![(240, 600), (243, 300), (246, 0)], true);
        assert!(d.percent().unwrap() > 90.0);
        assert_eq!(d.minutes_to_full(), None);
    }

    #[test]
    fn time_to_full_is_rounded_to_five_minutes() {
        let d = dev_with(vec![(128, 600), (140, 300), (153, 0)], true);
        let m = d.minutes_to_full().expect("should estimate");
        assert_eq!(m % 5, 0, "got {m}");
    }
}
