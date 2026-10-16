
//! PC mode: the pad shows up as an XInput device, which Windows.Gaming.Input.Custom
//! cannot reach. Its vendor HID collection carries the same power reads as a
//! 90-byte feature report (report id 4). Unlike the GIP path this works from
//! the background.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use windows::core::PCWSTR;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_Device_Interface_ListW, CM_Get_Device_Interface_List_SizeW,
    CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CR_SUCCESS,
};
use windows::Win32::Devices::HumanInterfaceDevice::{HidD_GetFeature, HidD_GetHidGuid, HidD_SetFeature};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};

use crate::protocol::{self, PowerReply, HID_REPORT_LEN};
use crate::state;

const REPORT_ID: u8 = 0x04;
const STATUS_NEW: u8 = 0x00;
const STATUS_BUSY: u8 = 0x01;
/// Interface path of the vendor collection in PC mode (USB id 1532:0A4C).
const PATH_MATCH: &str = "vid_1532&pid_0a4c&mi_01&col04";
/// GIP keys count up from 1, so this never collides.
const KEY: u64 = u64::MAX;

static BUSY: AtomicBool = AtomicBool::new(false);

/// Reads battery and charging on a worker thread; the HID round trip blocks.
pub fn request_refresh() {
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        poll();
        BUSY.store(false, Ordering::SeqCst);
    });
}

fn poll() {
    // With the pad off the dongle drops off USB entirely in PC mode, so a
    // missing interface is the disconnect signal.
    let Some(handle) = find_path().and_then(|p| open(&p)) else {
        state::set_connected(KEY, false);
        return;
    };
    let battery = ask(handle, protocol::CMD_BATTERY);
    let charging = battery.and_then(|_| ask(handle, protocol::CMD_CHARGING));
    unsafe {
        let _ = CloseHandle(handle);
    }
    // No usable reply keeps the last reading; the panel shows its age.
    let Some(PowerReply::Battery(raw)) = battery else {
        return;
    };
    // Same pad, same 0-255 value as Xbox mode, so it shares that card and history.
    let (vid, pid, _) = state::KNOWN[0];
    state::upsert(KEY, vid, pid, crate::link::detect_pc_mode(vid));
    state::record_battery(KEY, raw);
    if let Some(PowerReply::Charging(c)) = charging {
        state::record_charging(KEY, c);
    }
}

/// None when there is no usable reply this time, e.g. Synapse's request crossed ours.
fn ask(handle: HANDLE, cmd_id: u8) -> Option<PowerReply> {
    let mut buf = [0u8; HID_REPORT_LEN + 1];
    buf[0] = REPORT_ID;
    buf[1..].copy_from_slice(&protocol::hid_request(cmd_id));
    // Two sends at most: Synapse shares this report, and if its request lands
    // between our set and get we read its reply instead of ours.
    for _ in 0..2 {
        unsafe {
            if !HidD_SetFeature(handle, buf.as_ptr() as _, buf.len() as u32).as_bool() {
                return None;
            }
            for _ in 0..10 {
                std::thread::sleep(Duration::from_millis(50));
                let mut reply = [0u8; HID_REPORT_LEN + 1];
                reply[0] = REPORT_ID;
                if !HidD_GetFeature(handle, reply.as_mut_ptr() as _, reply.len() as u32).as_bool() {
                    return None;
                }
                if reply[1] == STATUS_NEW || reply[1] == STATUS_BUSY {
                    continue;
                }
                // Another transaction id or command: not our reply, send again.
                match protocol::parse_report(&reply[1..]) {
                    Some(r @ PowerReply::Battery(_)) if cmd_id == protocol::CMD_BATTERY => return Some(r),
                    Some(r @ PowerReply::Charging(_)) if cmd_id == protocol::CMD_CHARGING => return Some(r),
                    _ => break,
                }
            }
        }
    }
    None
}

fn find_path() -> Option<Vec<u16>> {
    unsafe {
        let guid = HidD_GetHidGuid();
        let mut len = 0u32;
        if CM_Get_Device_Interface_List_SizeW(&mut len, &guid, PCWSTR::null(), CM_GET_DEVICE_INTERFACE_LIST_PRESENT)
            != CR_SUCCESS
        {
            return None;
        }
        let mut list = vec![0u16; len as usize];
        if CM_Get_Device_Interface_ListW(&guid, PCWSTR::null(), &mut list, CM_GET_DEVICE_INTERFACE_LIST_PRESENT)
            != CR_SUCCESS
        {
            return None;
        }
        list.split(|c| *c == 0)
            .find(|p| String::from_utf16_lossy(p).to_ascii_lowercase().contains(PATH_MATCH))
            .map(|p| p.iter().copied().chain([0]).collect())
    }
}

fn open(path: &[u16]) -> Option<HANDLE> {
    unsafe {
        CreateFileW(
            PCWSTR(path.as_ptr()),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
        .ok()
    }
}
