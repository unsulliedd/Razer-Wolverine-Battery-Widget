//! Light/dark handling and the HKCU bits the app owns.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{ERROR_SUCCESS, MAX_PATH};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_BINARY,
    RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};

const PERSONALIZE: PCWSTR =
    w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
/// Task Manager's "Startup apps" tab does not delete the Run value when you
/// switch an entry off. It records the decision here instead, under the same
/// value name, and Explorer consults it at logon.
const APPROVED_KEY: PCWSTR =
    w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run");
const APP_VALUE: PCWSTR = w!("WolverineBattery");

/// The taskbar follows SystemUsesLightTheme, a different setting from the app
/// theme. A missing value means dark, which is the Windows default.
pub fn taskbar_is_light() -> bool {
    unsafe {
        let mut value: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        let rc = RegGetValueW(
            HKEY_CURRENT_USER,
            PERSONALIZE,
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        );
        rc == ERROR_SUCCESS && value != 0
    }
}

fn exe_path() -> String {
    unsafe {
        let mut buf = [0u16; MAX_PATH as usize];
        let n = GetModuleFileNameW(None, &mut buf);
        String::from_utf16_lossy(&buf[..n as usize])
    }
}

/// The command stored in the Run key, or None when there is no entry.
fn autostart_command() -> Option<String> {
    unsafe {
        let mut size = 0u32;
        if RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            APP_VALUE,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        ) != ERROR_SUCCESS
        {
            return None;
        }
        let mut buf = vec![0u16; size as usize / 2 + 1];
        if RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            APP_VALUE,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        ) != ERROR_SUCCESS
        {
            return None;
        }
        let n = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..n]))
    }
}

/// Whether a Run command launches this exact executable.
fn targets(command: &str, exe: &str) -> bool {
    command.trim().trim_matches('"').eq_ignore_ascii_case(exe)
}

/// The first byte of the StartupApproved record, if Windows has written one.
///
/// The record is twelve bytes - a flag, then a timestamp nobody needs - but
/// only the length is documented by observation, so reading a short one is
/// treated as a record rather than as no record at all.
fn approval_flag() -> Option<u8> {
    unsafe {
        let mut buf = [0u8; 12];
        let mut size = buf.len() as u32;
        let rc = RegGetValueW(
            HKEY_CURRENT_USER,
            APPROVED_KEY,
            APP_VALUE,
            RRF_RT_REG_BINARY,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        );
        (rc == ERROR_SUCCESS && size >= 1).then_some(buf[0])
    }
}

/// Explorer stores the decision in the low bit: 0x02 and 0x06 are enabled, 0x03
/// and 0x09 are disabled. No record at all means enabled, which is the state of
/// every entry nobody has ever switched off.
fn approval_says_disabled(flag: Option<u8>) -> bool {
    flag.is_some_and(|b| b & 1 == 1)
}

/// Drop the StartupApproved record so the Run key alone decides.
///
/// Deleting rather than rewriting it: absent is the state Windows ships with,
/// and it cannot disagree with itself the way a hand-built byte string can.
fn clear_approval() {
    unsafe {
        let mut key = HKEY::default();
        // Open, never create. If the key is absent there is nothing recorded to
        // clear, and conjuring it up would be writing to a part of the registry
        // that is Explorer's, not ours.
        if RegOpenKeyExW(HKEY_CURRENT_USER, APPROVED_KEY, 0, KEY_SET_VALUE, &mut key)
            != ERROR_SUCCESS
        {
            return;
        }
        let _ = RegDeleteValueW(key, APP_VALUE);
        let _ = RegCloseKey(key);
    }
}

/// True only when the Run entry points at *this* executable **and** Windows has
/// not been told to skip it.
///
/// An entry that names some other binary is not autostart working, it is
/// Windows launching something else - so reporting it as enabled tells the user
/// their controller will be waiting for them when it will not. Switching the
/// entry off in Task Manager leaves the Run value sitting there untouched and
/// produces exactly the same lie, so both have to be checked.
pub fn autostart_enabled() -> bool {
    match autostart_command() {
        Some(cmd) => targets(&cmd, &exe_path()) && !approval_says_disabled(approval_flag()),
        None => false,
    }
}

/// Repoint a stale entry at the current executable.
///
/// The entry existing is the user's intent. Renaming or moving the binary must
/// not silently turn that intent into a no-op, which is what happened when the
/// bin was renamed and the Run key went on naming the old file.
///
/// Only the path is repaired, never whether the entry is switched on. Someone
/// who turned it off in Task Manager meant it, and that record is left alone.
pub fn repair_autostart() {
    let exe = exe_path();
    if autostart_command().is_some_and(|cmd| !targets(&cmd, &exe)) {
        write_run_value(true);
    }
}

pub fn set_autostart(on: bool) {
    write_run_value(on);
    // Either way the Run key now says all there is to say. A leftover
    // StartupApproved record could only contradict it.
    clear_approval();
}

fn write_run_value(on: bool) {
    unsafe {
        let mut key = HKEY::default();
        if RegCreateKeyExW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            0,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        ) != ERROR_SUCCESS
        {
            return;
        }
        if on {
            let quoted = format!("\"{}\"", exe_path());
            let mut wide: Vec<u16> = quoted.encode_utf16().collect();
            wide.push(0);
            let bytes = std::slice::from_raw_parts(
                wide.as_ptr() as *const u8,
                wide.len() * 2,
            );
            let _ = RegSetValueExW(key, APP_VALUE, 0, REG_SZ, Some(bytes));
        } else {
            let _ = RegDeleteValueW(key, APP_VALUE);
        }
        let _ = RegCloseKey(key);
    }
}

#[cfg(test)]
mod tests {
    use super::{approval_says_disabled, targets};

    const EXE: &str = r"C:\Tools\WolverineBatteryWidget.exe";

    #[test]
    fn a_quoted_command_matches_its_own_exe() {
        assert!(targets(&format!("\"{EXE}\""), EXE));
        assert!(targets(EXE, EXE));
        assert!(targets(&format!("  \"{EXE}\"  "), EXE));
        // The Run key is written by Windows tooling as often as by us, so the
        // case it comes back in is not ours to assume.
        assert!(targets(&EXE.to_uppercase(), EXE));
    }

    #[test]
    fn a_command_naming_another_binary_does_not_match() {
        // The exact shape of the bug: the bin was renamed and the Run key went
        // on naming the old file, which still existed and still launched.
        assert!(!targets("\"C:\\Tools\\WolverineBattery.exe\"", EXE));
        assert!(!targets("\"C:\\Elsewhere\\WolverineBatteryWidget.exe\"", EXE));
        assert!(!targets("", EXE));
    }

    #[test]
    fn no_startup_approved_record_means_enabled() {
        // The state of every Run entry nobody has ever opened Task Manager for.
        assert!(!approval_says_disabled(None));
    }

    #[test]
    fn the_low_bit_of_the_flag_decides() {
        // Written by Explorer on first sight of the entry, and after a re-enable.
        assert!(!approval_says_disabled(Some(0x02)));
        assert!(!approval_says_disabled(Some(0x06)));
        // The case this whole check exists for: switched off in Task Manager,
        // Run value still present and still naming us.
        assert!(approval_says_disabled(Some(0x03)));
        assert!(approval_says_disabled(Some(0x09)));
    }
}
