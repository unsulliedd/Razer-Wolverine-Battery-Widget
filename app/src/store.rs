//! Last-known readings and sample history, persisted to %LOCALAPPDATA%.
//!
//! Devices are separated by `|`, the history from the head by `#`, points by
//! `;`. The markers cannot be letters: `h` collided with the `h` in `charging`
//! and silently truncated every record.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::state::Sample;

#[derive(Clone, Debug)]
pub struct Entry {
    pub vid: u16,
    pub pid: u16,
    pub raw: u8,
    pub charging: bool,
    pub at: SystemTime,
    pub history: Vec<Sample>,
}

static LOADED: Mutex<Vec<Entry>> = Mutex::new(Vec::new());

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    let dir = PathBuf::from(base).join("WolverineBattery");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("state.json"))
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// Pull `"key":<digits>` out of an object body.
fn num(body: &str, key: &str) -> Option<u64> {
    let i = body.find(&format!("\"{key}\""))?;
    let rest = &body[i + key.len() + 2..];
    let rest = rest.trim_start_matches([':', ' ']);
    let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// Read the file once at startup. A missing or corrupt file is not an error.
pub fn load() {
    let Some(p) = path() else { return };
    let Ok(text) = std::fs::read_to_string(&p) else {
        return;
    };
    let mut out: Vec<Entry> = Vec::new();
    for block in text.split('|').skip(1) {
        let head = block.split('#').next().unwrap_or("");
        let (Some(vid), Some(pid), Some(raw), Some(at)) = (
            num(head, "vid"),
            num(head, "pid"),
            num(head, "raw"),
            num(head, "at"),
        ) else {
            continue;
        };
        if raw > 255 || vid > 0xFFFF || pid > 0xFFFF {
            continue;
        }
        let mut history = Vec::new();
        if let Some(hi) = block.find('#') {
            for point in block[hi + 1..].split(';') {
                let mut it = point.split(',');
                if let (Some(r), Some(t)) = (it.next(), it.next()) {
                    if let (Ok(r), Ok(t)) = (r.trim().parse::<u16>(), t.trim().parse::<u64>()) {
                        if r <= 255 {
                            history.push(Sample {
                                raw: r as u8,
                                at: UNIX_EPOCH + Duration::from_secs(t),
                            });
                        }
                    }
                }
            }
        }
        out.push(Entry {
            vid: vid as u16,
            pid: pid as u16,
            raw: raw as u8,
            charging: head.contains("\"charging\":true"),
            at: UNIX_EPOCH + Duration::from_secs(at),
            history,
        });
    }
    if let Ok(mut g) = LOADED.lock() {
        *g = out;
    }
}

pub fn last_known(vid: u16, pid: u16) -> Option<Entry> {
    LOADED
        .lock()
        .ok()?
        .iter()
        .find(|e| e.vid == vid && e.pid == pid)
        .cloned()
}

pub fn save(entries: &[Entry]) {
    let Some(p) = path() else { return };
    let mut s = String::from("{\"version\":2}");
    for e in entries {
        s.push('|');
        s.push_str(&format!(
            "{{\"vid\":{},\"pid\":{},\"raw\":{},\"charging\":{},\"at\":{}}}",
            e.vid,
            e.pid,
            e.raw,
            e.charging,
            secs(e.at)
        ));
        if !e.history.is_empty() {
            s.push('#');
            let points: Vec<String> = e
                .history
                .iter()
                .map(|p| format!("{},{}", p.raw, secs(p.at)))
                .collect();
            s.push_str(&points.join(";"));
        }
    }
    // Write then rename, so a crash mid-write cannot leave a truncated file.
    let tmp = p.with_extension("tmp");
    if std::fs::write(&tmp, s).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `LOCALAPPDATA` and `LOADED` are process-wide, so the tests that touch
    /// them cannot run concurrently.
    static STORE: Mutex<()> = Mutex::new(());

    fn sample(raw: u8, t: u64) -> Sample {
        Sample {
            raw,
            at: UNIX_EPOCH + Duration::from_secs(t),
        }
    }

    /// Point the store at an empty private directory and return it.
    fn sandbox(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("LOCALAPPDATA", &dir);
        dir
    }

    #[test]
    fn save_then_load_round_trips() {
        let _guard = STORE.lock().unwrap_or_else(|e| e.into_inner());
        sandbox("wolverine-store-roundtrip");

        let entry = Entry {
            vid: 0x1532,
            pid: 0x0A3F,
            raw: 186,
            charging: true,
            at: UNIX_EPOCH + Duration::from_secs(1_750_000_000),
            history: vec![sample(180, 1_749_999_000), sample(186, 1_750_000_000)],
        };
        save(std::slice::from_ref(&entry));
        load();

        let got = last_known(0x1532, 0x0A3F).expect("entry survives the round trip");
        assert_eq!(got.raw, 186);
        assert!(got.charging);
        assert_eq!(got.history.len(), 2);
        assert_eq!(got.history[1].raw, 186);
        assert_eq!(secs(got.at), 1_750_000_000);
    }

    #[test]
    fn a_corrupt_value_is_dropped_rather_than_clamped() {
        let _guard = STORE.lock().unwrap_or_else(|e| e.into_inner());
        let dir = sandbox("wolverine-store-corrupt").join("WolverineBattery");
        std::fs::create_dir_all(&dir).unwrap();
        // 999 is not a battery level. It must not become 255, or 231, or
        // anything else that would render as a plausible percentage. The
        // second record is intact, so a bad one must not take the file with it.
        std::fs::write(
            dir.join("state.json"),
            "{\"version\":2}\
             |{\"vid\":5426,\"pid\":2623,\"raw\":999,\"charging\":false,\"at\":1750000000}\
             |{\"vid\":5426,\"pid\":2624,\"raw\":186,\"charging\":false,\"at\":1750000000}\
             #300,1749999000;186,1750000000",
        )
        .unwrap();
        load();

        assert!(
            last_known(0x1532, 0x0A3F).is_none(),
            "raw 999 must be dropped, not clamped into range"
        );
        let intact = last_known(0x1532, 0x0A40).expect("the valid record still loads");
        assert_eq!(intact.raw, 186);
        assert_eq!(
            intact.history.len(),
            1,
            "the out-of-range history point must be dropped too"
        );
        assert_eq!(intact.history[0].raw, 186);
    }
}
