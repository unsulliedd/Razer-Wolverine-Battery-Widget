//! The Razer vendor report as it appears on this device's GIP transport.
//!
//! Every constant here was observed on the wire. The tests at the bottom run
//! against real captured frames.

/// 60 bytes on this device; Razer's desktop peripherals use 90.
pub const REPORT_LEN: usize = 60;

/// The transport builds and strips the 4-byte GIP header itself, so these only
/// identify a message rather than frame one.
pub const GIP_REQUEST_ID: u8 = 0x0F;
pub const GIP_REPLY_ID: u8 = 0x10;

const STATUS_REQUEST: u8 = 0x00;
const STATUS_OK: u8 = 0x02;
const TRANSACTION_ID: u8 = 0x1F;

pub const CLASS_POWER: u8 = 0x07;
pub const CMD_BATTERY: u8 = 0x80;
pub const CMD_CHARGING: u8 = 0x84;

/// XOR over bytes 2..58 of the report.
fn checksum(report: &[u8]) -> u8 {
    report[2..REPORT_LEN - 2].iter().fold(0u8, |a, b| a ^ b)
}

/// Command ids without the high bit set are writes on this device, so only the
/// two power reads are exposed.
fn build(cmd_class: u8, cmd_id: u8) -> [u8; REPORT_LEN] {
    let mut r = [0u8; REPORT_LEN];
    r[0] = STATUS_REQUEST;
    r[1] = TRANSACTION_ID;
    r[5] = 0x02; // data size
    r[6] = cmd_class;
    r[7] = cmd_id;
    r[REPORT_LEN - 2] = checksum(&r);
    r
}

pub fn battery_request() -> [u8; REPORT_LEN] {
    build(CLASS_POWER, CMD_BATTERY)
}

pub fn charging_request() -> [u8; REPORT_LEN] {
    build(CLASS_POWER, CMD_CHARGING)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerReply {
    /// Raw 0-255 value straight off the wire, never a percentage.
    Battery(u8),
    Charging(bool),
}

/// Linearity at the extremes is unverified; observed values span roughly
/// 145-237.
pub fn percent(raw: u8) -> f32 {
    raw as f32 * 100.0 / 255.0
}

/// Parse a message arriving on the GIP input sink. Every failure path returns
/// `None` and never a number.
pub fn parse_reply(message_id: u8, buf: &[u8]) -> Option<PowerReply> {
    if message_id != GIP_REPLY_ID {
        return None;
    }
    if buf.len() < REPORT_LEN {
        return None;
    }
    let report = &buf[..REPORT_LEN];
    if report[0] != STATUS_OK || report[1] != TRANSACTION_ID {
        return None;
    }
    if checksum(report) != report[REPORT_LEN - 2] {
        return None;
    }
    if report[6] != CLASS_POWER {
        return None;
    }
    match report[7] {
        CMD_BATTERY => Some(PowerReply::Battery(report[9])),
        CMD_CHARGING => Some(PowerReply::Charging(report[9] != 0)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Expand a hex string into a 60-byte report, zero padded, with the
    /// checksum at 58 as it appears on the wire.
    fn report(prefix: &str, checksum_byte: u8) -> Vec<u8> {
        let mut v: Vec<u8> = prefix
            .split_whitespace()
            .map(|h| u8::from_str_radix(h, 16).unwrap())
            .collect();
        v.resize(REPORT_LEN, 0);
        v[REPORT_LEN - 2] = checksum_byte;
        v
    }

    #[test]
    fn request_matches_the_bytes_we_put_on_the_wire() {
        // Captured: 0f00e03c 001f000000020780 00 ... 8500
        let req = battery_request();
        assert_eq!(&req[..8], &[0x00, 0x1F, 0x00, 0x00, 0x00, 0x02, 0x07, 0x80]);
        assert_eq!(req[REPORT_LEN - 2], 0x85, "checksum seen on the wire");
        assert_eq!(req[REPORT_LEN - 1], 0x00);
    }

    #[test]
    fn charging_request_checksum() {
        let req = charging_request();
        assert_eq!(req[7], CMD_CHARGING);
        assert_eq!(req[REPORT_LEN - 2], 0x02 ^ 0x07 ^ 0x84);
    }

    #[test]
    fn parses_the_captured_battery_reply() {
        // Reply 0x10, value 0xED, checksum 0x68.
        let r = report("02 1F 00 00 00 02 07 80 00 ED", 0x68);
        assert_eq!(
            parse_reply(GIP_REPLY_ID, &r),
            Some(PowerReply::Battery(0xED))
        );
        assert!((percent(0xED) - 92.9).abs() < 0.1);
    }

    #[test]
    fn parses_razers_captured_battery_reply() {
        // From a capture of Razer's own app: 0x91, checksum 0x14.
        let r = report("02 1F 00 00 00 02 07 80 00 91", 0x14);
        assert_eq!(
            parse_reply(GIP_REPLY_ID, &r),
            Some(PowerReply::Battery(0x91))
        );
        assert!((percent(0x91) - 56.9).abs() < 0.1);
    }

    #[test]
    fn parses_the_captured_charging_reply() {
        let r = report("02 1F 00 00 00 02 07 84 00 01", 0x80);
        assert_eq!(
            parse_reply(GIP_REPLY_ID, &r),
            Some(PowerReply::Charging(true))
        );
    }

    #[test]
    fn rejects_a_bad_checksum() {
        let mut r = report("02 1F 00 00 00 02 07 80 00 ED", 0x68);
        r[REPORT_LEN - 2] ^= 0xFF;
        assert_eq!(parse_reply(GIP_REPLY_ID, &r), None);
    }

    #[test]
    fn rejects_a_failed_status() {
        let mut r = report("02 1F 00 00 00 02 07 80 00 ED", 0x68);
        r[0] = 0x03;
        assert_eq!(parse_reply(GIP_REPLY_ID, &r), None);
    }

    #[test]
    fn rejects_the_wrong_message_id() {
        let r = report("02 1F 00 00 00 02 07 80 00 ED", 0x68);
        assert_eq!(parse_reply(GIP_REQUEST_ID, &r), None);
    }

    #[test]
    fn rejects_the_unsolicited_power_notification() {
        // Class 0x0C cmd 0xF0, seen once during probing. Shape unconfirmed.
        let r = report("00 00 00 00 00 09 0C F0 08 BC 01", 0x00);
        assert_eq!(parse_reply(GIP_REPLY_ID, &r), None);
    }

    #[test]
    fn rejects_a_truncated_buffer() {
        let r = vec![0x02, 0x1F, 0x00, 0x00, 0x00, 0x02, 0x07, 0x80, 0x00, 0xED];
        assert_eq!(parse_reply(GIP_REPLY_ID, &r), None);
    }

    #[test]
    fn rejects_an_all_zero_buffer() {
        let r = vec![0u8; REPORT_LEN];
        assert_eq!(parse_reply(GIP_REPLY_ID, &r), None);
    }
}
