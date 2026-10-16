
//! Low-battery alerts, once per threshold crossing.

use std::sync::atomic::{AtomicU8, Ordering};

const THRESHOLDS: [u8; 2] = [20, 10];
/// Charging readings run high, so only a real recovery re-arms the alerts.
const REARM_ABOVE: u8 = 25;
const NONE: u8 = u8::MAX;

/// Lowest threshold already announced, or NONE.
static ALERTED: AtomicU8 = AtomicU8::new(NONE);

/// The threshold to announce for this reading, if any.
pub fn check(pct: u8, charging: bool) -> Option<u8> {
    let (next, fire) = step(ALERTED.load(Ordering::Relaxed), pct, charging);
    ALERTED.store(next, Ordering::Relaxed);
    fire
}

fn step(alerted: u8, pct: u8, charging: bool) -> (u8, Option<u8>) {
    if pct > REARM_ABOVE {
        return (NONE, None);
    }
    if charging {
        return (alerted, None);
    }
    match THRESHOLDS.iter().copied().filter(|t| pct <= *t).min() {
        Some(t) if t < alerted => (t, Some(t)),
        _ => (alerted, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed readings in order, collecting what would be announced.
    fn run(readings: &[(u8, bool)]) -> Vec<u8> {
        let mut alerted = NONE;
        let mut fired = Vec::new();
        for (pct, charging) in readings {
            let (next, fire) = step(alerted, *pct, *charging);
            alerted = next;
            fired.extend(fire);
        }
        fired
    }

    #[test]
    fn each_threshold_fires_once_on_the_way_down() {
        assert_eq!(run(&[(30, false), (20, false), (19, false), (15, false), (10, false), (5, false)]), [20, 10]);
    }

    #[test]
    fn starting_below_both_announces_the_lower_one_only() {
        assert_eq!(run(&[(8, false), (7, false)]), [10]);
    }

    #[test]
    fn plugging_in_briefly_does_not_rearm() {
        // Inflated reading on the cable, then back to the real level.
        assert_eq!(run(&[(18, false), (23, true), (19, false)]), [20]);
    }

    #[test]
    fn recovering_above_the_rearm_level_rearms() {
        assert_eq!(run(&[(18, false), (40, true), (40, false), (18, false)]), [20, 20]);
    }

    #[test]
    fn nothing_while_charging() {
        assert_eq!(run(&[(9, true), (12, true)]), Vec::<u8>::new());
    }
}
