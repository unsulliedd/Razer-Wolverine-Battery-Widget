//! The transport: Windows.Gaming.Input.Custom, from an unpackaged process.
//!
//! Four device facts this file depends on:
//!
//!   1. The process needs a window and a running message pump, or Windows
//!      never enumerates any controller at all.
//!   2. The object returned from CreateGameController must answer
//!      {06E58977-7684-4DC5-BAD1-CDA52A4AA06D}, an undocumented interface that
//!      appears in no SDK header. Refuse it and Windows silently abandons the
//!      controller: no OnGameControllerAdded, no vendor messaging, no error.
//!   3. SendReceiveMessage never works on this device. Use SendMessage and take
//!      the reply asynchronously on the input sink, with message class Command;
//!      the other two classes are accepted and silently do nothing.
//!   4. Windows.Gaming.Input suspends the input sink whenever this process is
//!      not foreground. SendMessage keeps returning success and the reply never
//!      arrives.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

use windows::core::{implement, IInspectable, Interface, Result, GUID, HRESULT, HSTRING};
use windows::Gaming::Input::Custom::{
    GameControllerFactoryManager, GipGameControllerProvider, GipMessageClass,
    ICustomGameControllerFactory, ICustomGameControllerFactory_Impl, IGameControllerProvider,
};
use windows::Gaming::Input::IGameController;
use windows::Win32::Foundation::{E_NOINTERFACE, E_NOTIMPL, E_POINTER};
use windows::Win32::System::Com::CoTaskMemAlloc;

use crate::protocol::{self, PowerReply};
use crate::state;

const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
const IID_IINSPECTABLE: GUID = GUID::from_u128(0xaf86e2e0_b12d_4c6a_9c5a_d7aa65101e90);
const IID_IAGILEOBJECT: GUID = GUID::from_u128(0x94ea2b94_e9cc_49e0_c0ff_ee64ca8f5b90);
const IID_SINK: GUID = GUID::from_u128(0x1ff6f922_c640_4c78_a820_9a715c558bcb);
const IID_GIP_SINK: GUID = GUID::from_u128(0xa2108abf_09f1_43bc_a140_80f899ec36fb);

/// Undocumented; queried before anything else and fatal if refused. Slot 6 is
/// the only method Windows ever calls.
const IID_GAME_CONTROLLER_IMPL: GUID = GUID::from_u128(0x06e58977_7684_4dc5_bad1_cda52a4aa06d);

struct SendPtr(GipGameControllerProvider);
// WinRT controller providers are agile.
unsafe impl Send for SendPtr {}

/// Keyed by model like the state registry, so a reconnect replaces the stale
/// provider instead of stacking a second one behind it.
static PROVIDERS: Mutex<Vec<(u64, u16, u16, SendPtr)>> = Mutex::new(Vec::new());
static NEXT_KEY: AtomicU64 = AtomicU64::new(1);

static EVER_ACTIVATED: AtomicBool = AtomicBool::new(false);

pub fn transport_ever_worked() -> bool {
    EVER_ACTIVATED.load(Ordering::SeqCst)
}

// The controller object is hand-rolled rather than #[implement]-generated: it
// has to expose an interface windows-rs has never heard of.

#[repr(C)]
struct InspVtbl {
    query: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    get_iids: unsafe extern "system" fn(*mut c_void, *mut u32, *mut *mut GUID) -> HRESULT,
    get_name: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    get_trust: unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT,
}

#[repr(C)]
struct SinkVtbl {
    base: InspVtbl,
    on_input_resumed: unsafe extern "system" fn(*mut c_void, u64) -> HRESULT,
    on_input_suspended: unsafe extern "system" fn(*mut c_void, u64) -> HRESULT,
}

#[repr(C)]
struct GipVtbl {
    base: InspVtbl,
    on_key_received: unsafe extern "system" fn(*mut c_void, u64, u8, u8) -> HRESULT,
    on_message_received:
        unsafe extern "system" fn(*mut c_void, u64, i32, u8, u8, u32, *const u8) -> HRESULT,
}

#[repr(C)]
struct ImplVtbl {
    base: InspVtbl,
    initialize: unsafe extern "system" fn(*mut c_void, usize, usize, usize) -> HRESULT,
    unused: [unsafe extern "system" fn(*mut c_void, usize, usize, usize) -> HRESULT; 4],
}

#[repr(C)]
struct Controller {
    v_insp: *const InspVtbl,
    v_sink: *const SinkVtbl,
    v_gip: *const GipVtbl,
    v_impl: *const ImplVtbl,
    refs: AtomicU32,
    key: u64,
}

const OFF_INSP: isize = 0;
const OFF_SINK: isize = 8;
const OFF_GIP: isize = 16;
const OFF_IMPL: isize = 24;

unsafe fn base_of(this: *mut c_void, off: isize) -> *mut Controller {
    (this as *mut u8).offset(-off) as *mut Controller
}

unsafe fn do_query(obj: *mut Controller, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if iid.is_null() || out.is_null() {
        return E_POINTER;
    }
    let iid = *iid;
    let ptr: *mut c_void =
        if iid == IID_IUNKNOWN || iid == IID_IINSPECTABLE || iid == IID_IAGILEOBJECT {
            obj as *mut c_void
        } else if iid == IID_SINK {
            (obj as *mut u8).offset(OFF_SINK) as *mut c_void
        } else if iid == IID_GIP_SINK {
            (obj as *mut u8).offset(OFF_GIP) as *mut c_void
        } else if iid == IID_GAME_CONTROLLER_IMPL {
            (obj as *mut u8).offset(OFF_IMPL) as *mut c_void
        } else {
            std::ptr::null_mut()
        };
    if ptr.is_null() {
        *out = std::ptr::null_mut();
        return E_NOINTERFACE;
    }
    (*obj).refs.fetch_add(1, Ordering::SeqCst);
    *out = ptr;
    HRESULT(0)
}

unsafe fn do_release(obj: *mut Controller) -> u32 {
    let n = (*obj).refs.fetch_sub(1, Ordering::SeqCst) - 1;
    if n == 0 {
        let key = (*obj).key;
        forget_provider(key);
        state::set_connected(key, false);
        drop(Box::from_raw(obj));
    }
    n
}

macro_rules! insp_thunks {
    ($q:ident, $a:ident, $r:ident, $gi:ident, $gn:ident, $gt:ident, $off:expr) => {
        unsafe extern "system" fn $q(
            this: *mut c_void,
            iid: *const GUID,
            out: *mut *mut c_void,
        ) -> HRESULT {
            do_query(base_of(this, $off), iid, out)
        }
        unsafe extern "system" fn $a(this: *mut c_void) -> u32 {
            let o = base_of(this, $off);
            (*o).refs.fetch_add(1, Ordering::SeqCst) + 1
        }
        unsafe extern "system" fn $r(this: *mut c_void) -> u32 {
            do_release(base_of(this, $off))
        }
        unsafe extern "system" fn $gi(
            _this: *mut c_void,
            count: *mut u32,
            out: *mut *mut GUID,
        ) -> HRESULT {
            let iids = [IID_SINK, IID_GIP_SINK];
            let mem = CoTaskMemAlloc(std::mem::size_of_val(&iids)) as *mut GUID;
            if mem.is_null() {
                return E_POINTER;
            }
            std::ptr::copy_nonoverlapping(iids.as_ptr(), mem, iids.len());
            *count = iids.len() as u32;
            *out = mem;
            HRESULT(0)
        }
        unsafe extern "system" fn $gn(_this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
            let h = HSTRING::from("WolverineBattery.Controller");
            *out = std::mem::transmute::<HSTRING, *mut c_void>(h);
            HRESULT(0)
        }
        unsafe extern "system" fn $gt(_this: *mut c_void, out: *mut i32) -> HRESULT {
            *out = 0; // BaseTrust
            HRESULT(0)
        }
    };
}

insp_thunks!(i_q, i_a, i_r, i_gi, i_gn, i_gt, OFF_INSP);
insp_thunks!(s_q, s_a, s_r, s_gi, s_gn, s_gt, OFF_SINK);
insp_thunks!(g_q, g_a, g_r, g_gi, g_gn, g_gt, OFF_GIP);
insp_thunks!(m_q, m_a, m_r, m_gi, m_gn, m_gt, OFF_IMPL);

unsafe extern "system" fn on_input_resumed(this: *mut c_void, _ts: u64) -> HRESULT {
    let key = (*base_of(this, OFF_SINK)).key;
    state::set_readable(key, true);
    // Foreground, so a read can succeed now. Do not wait for a timer.
    request_refresh();
    HRESULT(0)
}

unsafe extern "system" fn on_input_suspended(this: *mut c_void, _ts: u64) -> HRESULT {
    let key = (*base_of(this, OFF_SINK)).key;
    state::set_readable(key, false);
    HRESULT(0)
}

unsafe extern "system" fn on_key_received(_t: *mut c_void, _ts: u64, _k: u8, _p: u8) -> HRESULT {
    HRESULT(0)
}

unsafe extern "system" fn on_message_received(
    this: *mut c_void,
    _ts: u64,
    _class: i32,
    message_id: u8,
    _seq: u8,
    len: u32,
    buf: *const u8,
) -> HRESULT {
    let key = (*base_of(this, OFF_GIP)).key;
    if buf.is_null() || len == 0 {
        return HRESULT(0);
    }
    let slice = std::slice::from_raw_parts(buf, len as usize);
    match protocol::parse_reply(message_id, slice) {
        Some(PowerReply::Battery(raw)) => state::record_battery(key, raw),
        Some(PowerReply::Charging(c)) => state::record_charging(key, c),
        None => {}
    }
    HRESULT(0)
}

/// Slot 6 of the undocumented interface. The arguments must be left untouched:
/// the outer object is still under construction and re-entering its
/// QueryInterface from here faults.
unsafe extern "system" fn initialize(_t: *mut c_void, _a: usize, _b: usize, _c: usize) -> HRESULT {
    HRESULT(0)
}

unsafe extern "system" fn not_impl(_t: *mut c_void, _a: usize, _b: usize, _c: usize) -> HRESULT {
    E_NOTIMPL
}

static INSP_VTBL: InspVtbl = InspVtbl {
    query: i_q,
    add_ref: i_a,
    release: i_r,
    get_iids: i_gi,
    get_name: i_gn,
    get_trust: i_gt,
};
static SINK_VTBL: SinkVtbl = SinkVtbl {
    base: InspVtbl {
        query: s_q,
        add_ref: s_a,
        release: s_r,
        get_iids: s_gi,
        get_name: s_gn,
        get_trust: s_gt,
    },
    on_input_resumed,
    on_input_suspended,
};
static GIP_VTBL: GipVtbl = GipVtbl {
    base: InspVtbl {
        query: g_q,
        add_ref: g_a,
        release: g_r,
        get_iids: g_gi,
        get_name: g_gn,
        get_trust: g_gt,
    },
    on_key_received,
    on_message_received,
};
static IMPL_VTBL: ImplVtbl = ImplVtbl {
    base: InspVtbl {
        query: m_q,
        add_ref: m_a,
        release: m_r,
        get_iids: m_gi,
        get_name: m_gn,
        get_trust: m_gt,
    },
    initialize,
    unused: [not_impl; 4],
};

fn new_controller(key: u64) -> IInspectable {
    let obj = Box::into_raw(Box::new(Controller {
        v_insp: &INSP_VTBL,
        v_sink: &SINK_VTBL,
        v_gip: &GIP_VTBL,
        v_impl: &IMPL_VTBL,
        refs: AtomicU32::new(1),
        key,
    }));
    unsafe { IInspectable::from_raw(obj as *mut c_void) }
}

// ---------------------------------------------------------------------------

#[implement(ICustomGameControllerFactory)]
struct Factory;

impl ICustomGameControllerFactory_Impl for Factory_Impl {
    fn CreateGameController(
        &self,
        provider: Option<&IGameControllerProvider>,
    ) -> Result<IInspectable> {
        let key = NEXT_KEY.fetch_add(1, Ordering::SeqCst);
        if let Some(p) = provider {
            let vid = p.HardwareVendorId().unwrap_or(0);
            let pid = p.HardwareProductId().unwrap_or(0);
            state::upsert(key, vid, pid, crate::link::detect(vid, pid));
            if let Ok(gip) = p.cast::<GipGameControllerProvider>() {
                if let Ok(mut list) = PROVIDERS.lock() {
                    list.retain(|(_, v, d, _)| !(*v == vid && *d == pid));
                    list.push((key, vid, pid, SendPtr(gip)));
                }
            }
        }
        Ok(new_controller(key))
    }

    fn OnGameControllerAdded(&self, _value: Option<&IGameController>) -> Result<()> {
        EVER_ACTIVATED.store(true, Ordering::SeqCst);
        state::notify_ui();
        Ok(())
    }

    fn OnGameControllerRemoved(&self, _value: Option<&IGameController>) -> Result<()> {
        reconcile_connected();
        Ok(())
    }
}

fn forget_provider(key: u64) {
    if let Ok(mut list) = PROVIDERS.lock() {
        list.retain(|(k, _, _, _)| *k != key);
    }
}

/// Ask each provider whether it is still there. Driver state, not a hardware
/// round trip.
pub fn reconcile_connected() {
    let list = match PROVIDERS.lock() {
        Ok(l) => l,
        Err(_) => return,
    };
    for (key, vid, pid, SendPtr(p)) in list.iter() {
        let connected = p.IsConnected().unwrap_or(false);
        state::set_connected(*key, connected);
        if connected {
            // The pad can move between cable and dongle without the provider
            // ever being recreated.
            state::set_link(*key, crate::link::detect(*vid, *pid));
        }
    }
}

/// Ask every connected controller for battery and charging state. The replies
/// land on the input sink, or never, if the sink is suspended.
pub fn request_refresh() {
    let list = match PROVIDERS.lock() {
        Ok(l) => l,
        Err(_) => return,
    };
    let battery = protocol::battery_request();
    let charging = protocol::charging_request();
    for (_key, _vid, _pid, SendPtr(p)) in list.iter() {
        let _ = p.SendMessage(GipMessageClass::Command, protocol::GIP_REQUEST_ID, &battery);
        let _ = p.SendMessage(GipMessageClass::Command, protocol::GIP_REQUEST_ID, &charging);
    }
}

/// Must be called after the window exists and the message pump is running.
pub fn register() {
    let factory: ICustomGameControllerFactory = Factory.into();
    for (vid, pid, _) in state::KNOWN {
        let _ = GameControllerFactoryManager::RegisterCustomFactoryForHardwareId(
            &factory, *vid, *pid,
        );
    }
    // The factory must outlive this call; Windows holds it by pointer.
    std::mem::forget(factory);
}
