
use windows::core::PCWSTR;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW,
    SetupDiGetDeviceInstanceIdW, DIGCF_ALLCLASSES, DIGCF_PRESENT, SP_DEVINFO_DATA,
};

use crate::state::Link;

const DONGLE_SUFFIX: &str = "0000000000000001";

pub fn detect(vid: u16, pid: u16) -> Link {
    let prefix = format!("USB\\VID_{vid:04X}&PID_{pid:04X}\\");
    let mut wide: Vec<u16> = format!("USB\\VID_{vid:04X}&PID_{pid:04X}")
        .encode_utf16()
        .collect();
    wide.push(0);

    let mut result = Link::Unknown;
    unsafe {
        let set = match SetupDiGetClassDevsW(
            None,
            PCWSTR(wide.as_ptr()),
            None,
            DIGCF_PRESENT | DIGCF_ALLCLASSES,
        ) {
            Ok(s) => s,
            Err(_) => return Link::Unknown,
        };

        let mut index = 0u32;
        loop {
            let mut info = SP_DEVINFO_DATA {
                cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            if SetupDiEnumDeviceInfo(set, index, &mut info).is_err() {
                break;
            }
            index += 1;

            let mut buf = [0u16; 512];
            let mut needed = 0u32;
            if SetupDiGetDeviceInstanceIdW(set, &info, Some(&mut buf), Some(&mut needed)).is_err() {
                continue;
            }
            let id = String::from_utf16_lossy(&buf[..buf.iter().position(|c| *c == 0).unwrap_or(0)]);
            if !id.to_ascii_uppercase().starts_with(&prefix) {
                continue;
            }
            let suffix = &id[prefix.len()..];
            // A cabled pad wins if both nodes appear.
            if suffix.eq_ignore_ascii_case(DONGLE_SUFFIX) {
                if result == Link::Unknown {
                    result = Link::Dongle;
                }
            } else if !suffix.is_empty() {
                result = Link::Wired;
            }
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
    }
    result
}
