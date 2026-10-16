
use windows::core::PCWSTR;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW,
    SetupDiGetDeviceInstanceIdW, SetupDiGetDevicePropertyW, DIGCF_ALLCLASSES, DIGCF_PRESENT,
    HDEVINFO, SP_DEVINFO_DATA,
};
use windows::Win32::Devices::Properties::{DEVPKEY_Device_BusReportedDeviceDesc, DEVPROPTYPE};

use crate::state::Link;

const DONGLE_SUFFIX: &str = "0000000000000001";
/// PC mode (1532:0A4C) has the same device ids on cable and dongle; only the
/// USB product name differs: "...for Xbox" vs "...for Xbox 2.4".
const PC_MODE_PID: u16 = 0x0A4C;
const PC_MODE_DONGLE_NAME_END: &str = " 2.4";

/// Calls `f` with the part after `USB\VID_xxxx&PID_xxxx\` of each present
/// top-level USB node for this id.
fn each_usb_node(vid: u16, pid: u16, mut f: impl FnMut(HDEVINFO, &SP_DEVINFO_DATA, &str)) {
    let prefix = format!("USB\\VID_{vid:04X}&PID_{pid:04X}\\");
    let mut wide: Vec<u16> = format!("USB\\VID_{vid:04X}&PID_{pid:04X}")
        .encode_utf16()
        .collect();
    wide.push(0);

    unsafe {
        let set = match SetupDiGetClassDevsW(
            None,
            PCWSTR(wide.as_ptr()),
            None,
            DIGCF_PRESENT | DIGCF_ALLCLASSES,
        ) {
            Ok(s) => s,
            Err(_) => return,
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
            f(set, &info, &id[prefix.len()..]);
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
    }
}

pub fn detect(vid: u16, pid: u16) -> Link {
    let mut result = Link::Unknown;
    each_usb_node(vid, pid, |_, _, suffix| {
        // A cabled pad wins if both nodes appear.
        if suffix.eq_ignore_ascii_case(DONGLE_SUFFIX) {
            if result == Link::Unknown {
                result = Link::Dongle;
            }
        } else if !suffix.is_empty() {
            result = Link::Wired;
        }
    });
    result
}

pub fn detect_pc_mode(vid: u16) -> Link {
    let mut result = Link::Unknown;
    each_usb_node(vid, PC_MODE_PID, |set, info, _| unsafe {
        let mut kind = DEVPROPTYPE::default();
        let mut buf = [0u8; 512];
        let mut needed = 0u32;
        if SetupDiGetDevicePropertyW(
            set,
            info,
            &DEVPKEY_Device_BusReportedDeviceDesc,
            &mut kind,
            Some(&mut buf),
            Some(&mut needed),
            0,
        )
        .is_err()
        {
            return;
        }
        let wide: Vec<u16> = buf.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let name = String::from_utf16_lossy(&wide[..wide.iter().position(|c| *c == 0).unwrap_or(0)]);
        result = link_from_pc_name(&name);
    });
    result
}

fn link_from_pc_name(name: &str) -> Link {
    match name.trim() {
        "" => Link::Unknown,
        n if n.ends_with(PC_MODE_DONGLE_NAME_END) => Link::Dongle,
        _ => Link::Wired,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pc_mode_names_seen_on_this_pad() {
        assert_eq!(link_from_pc_name("Razer Wolverine V3 Pro for Xbox 2.4"), Link::Dongle);
        assert_eq!(link_from_pc_name("Razer Wolverine V3 Pro for Xbox"), Link::Wired);
        assert_eq!(link_from_pc_name(""), Link::Unknown);
    }
}
