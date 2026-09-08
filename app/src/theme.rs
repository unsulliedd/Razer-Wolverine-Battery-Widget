
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
const APPROVED_KEY: PCWSTR =
    w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run");
const APP_VALUE: PCWSTR = w!("WolverineBattery");
pub const AUTOSTART_ARG: &str = "--startup";

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

fn split_command(command: &str) -> (&str, &str) {
    let c = command.trim();
    if let Some(rest) = c.strip_prefix('"') {
        return match rest.find('"') {
            Some(i) => (&rest[..i], rest[i + 1..].trim()),
            None => (rest, ""),
        };
    }
    match c.find(AUTOSTART_ARG) {
        Some(i) => (c[..i].trim(), &c[i..]),
        None => (c, ""),
    }
}

/// Whether a Run command launches this exact executable.
fn targets(command: &str, exe: &str) -> bool {
    split_command(command).0.eq_ignore_ascii_case(exe)
}

/// Whether a Run command carries the flag that marks a logon launch.
fn has_autostart_arg(command: &str) -> bool {
    split_command(command)
        .1
        .split_whitespace()
        .any(|a| a.eq_ignore_ascii_case(AUTOSTART_ARG))
}

pub fn launched_at_logon() -> bool {
    std::env::args()
        .skip(1)
        .any(|a| a.eq_ignore_ascii_case(AUTOSTART_ARG))
}

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

fn approval_says_disabled(flag: Option<u8>) -> bool {
    flag.is_some_and(|b| b & 1 == 1)
}

fn clear_approval() {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, APPROVED_KEY, 0, KEY_SET_VALUE, &mut key)
            != ERROR_SUCCESS
        {
            return;
        }
        let _ = RegDeleteValueW(key, APP_VALUE);
        let _ = RegCloseKey(key);
    }
}

pub fn autostart_enabled() -> bool {
    match autostart_command() {
        Some(cmd) => targets(&cmd, &exe_path()) && !approval_says_disabled(approval_flag()),
        None => false,
    }
}

pub fn repair_autostart() {
    let exe = exe_path();
    if autostart_command().is_some_and(|cmd| !targets(&cmd, &exe) || !has_autostart_arg(&cmd)) {
        write_run_value(true);
    }
}

pub fn set_autostart(on: bool) {
    write_run_value(on);
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
            let quoted = format!("\"{}\" {AUTOSTART_ARG}", exe_path());
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
    use super::{approval_says_disabled, has_autostart_arg, targets, AUTOSTART_ARG};

    const EXE: &str = r"C:\Tools\WolverineBatteryWidget.exe";

    #[test]
    fn a_quoted_command_matches_its_own_exe() {
        assert!(targets(&format!("\"{EXE}\""), EXE));
        assert!(targets(EXE, EXE));
        assert!(targets(&format!("  \"{EXE}\"  "), EXE));
        assert!(targets(&EXE.to_uppercase(), EXE));
    }

    #[test]
    fn the_startup_flag_does_not_hide_the_exe() {
        let cmd = format!("\"{EXE}\" {AUTOSTART_ARG}");
        assert!(targets(&cmd, EXE));
        assert!(has_autostart_arg(&cmd));
    }

    #[test]
    fn an_entry_written_before_the_flag_existed_still_matches() {
        assert!(targets(&format!("\"{EXE}\""), EXE));
        assert!(!has_autostart_arg(&format!("\"{EXE}\"")));
    }

    #[test]
    fn a_space_in_the_path_survives_the_split() {
        const SPACED: &str = r"C:\Users\me\Razer Controller Widget\App.exe";
        assert!(targets(&format!("\"{SPACED}\" {AUTOSTART_ARG}"), SPACED));
        assert!(targets(&format!("{SPACED} {AUTOSTART_ARG}"), SPACED));
        assert!(targets(SPACED, SPACED));
    }

    #[test]
    fn a_command_naming_another_binary_does_not_match() {
        assert!(!targets("\"C:\\Tools\\WolverineBattery.exe\"", EXE));
        assert!(!targets("\"C:\\Elsewhere\\WolverineBatteryWidget.exe\"", EXE));
        assert!(!targets("", EXE));
    }

    #[test]
    fn no_startup_approved_record_means_enabled() {
        assert!(!approval_says_disabled(None));
    }

    #[test]
    fn the_low_bit_of_the_flag_decides() {
        assert!(!approval_says_disabled(Some(0x02)));
        assert!(!approval_says_disabled(Some(0x06)));
        assert!(approval_says_disabled(Some(0x03)));
        assert!(approval_says_disabled(Some(0x09)));
    }
}
