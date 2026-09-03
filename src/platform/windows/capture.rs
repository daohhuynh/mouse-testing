//! Device-level capture on Windows: Raw Input.
//!
//! Raw Input is the lowest level an unprivileged process can reach. Windows
//! opens every mouse top-level collection exclusively, so reading HID reports
//! directly is refused whether or not the process is elevated. This level is
//! therefore "the rate the OS received", not "the rate the mouse sent", and the
//! interface says so.
//!
//! The pump owns a message-only window on its own thread. Sharing the user
//! interface's message queue would put mouse reports behind repaints and turn a
//! mouse measurement into a measurement of the interface.

use crate::core::clock;
use crate::core::ring::{Consumer, Ring};
use crate::core::sample::{Flags, Kind, Sample};
use std::cell::Cell;
use std::ffi::c_void;
use std::mem::{size_of, zeroed, MaybeUninit};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{
    GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
};
use windows_sys::Win32::UI::Input::{
    GetRawInputData, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT,
    MOUSE_MOVE_ABSOLUTE, RIDEV_DEVNOTIFY, RIDEV_INPUTSINK, RIDEV_PAGEONLY, RIDEV_REMOVE,
    RIM_TYPEMOUSE, RegisterRawInputDevices,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    PostThreadMessageW, RegisterClassExW, UnregisterClassW, HWND_MESSAGE,
    MSG, RI_MOUSE_BUTTON_4_DOWN, RI_MOUSE_BUTTON_4_UP, RI_MOUSE_BUTTON_5_DOWN,
    RI_MOUSE_BUTTON_5_UP, RI_MOUSE_HWHEEL, RI_MOUSE_LEFT_BUTTON_DOWN, RI_MOUSE_LEFT_BUTTON_UP,
    RI_MOUSE_MIDDLE_BUTTON_DOWN, RI_MOUSE_MIDDLE_BUTTON_UP, RI_MOUSE_RIGHT_BUTTON_DOWN,
    RI_MOUSE_RIGHT_BUTTON_UP, RI_MOUSE_WHEEL, WM_APP, WM_INPUT, WNDCLASSEXW,
};

const WM_STOP_PUMP: u32 = WM_APP + 1;
/// `GET_RAWINPUT_CODE_WPARAM(w)` is `w & 0xff`; 0 means we were in the
/// foreground, 1 means the event reached us only because of RIDEV_INPUTSINK.
const RIM_INPUT: u32 = 0;

/// One bit per (button, direction) transition. Iterated in full, never with
/// `else`: a single report can carry several transitions at once, and an
/// if/else chain silently eats all but the first.
const BUTTON_TABLE: [(u16, u32, bool); 10] = [
    (RI_MOUSE_LEFT_BUTTON_DOWN as u16, 0, true),
    (RI_MOUSE_LEFT_BUTTON_UP as u16, 0, false),
    (RI_MOUSE_RIGHT_BUTTON_DOWN as u16, 1, true),
    (RI_MOUSE_RIGHT_BUTTON_UP as u16, 1, false),
    (RI_MOUSE_MIDDLE_BUTTON_DOWN as u16, 2, true),
    (RI_MOUSE_MIDDLE_BUTTON_UP as u16, 2, false),
    (RI_MOUSE_BUTTON_4_DOWN as u16, 3, true),
    (RI_MOUSE_BUTTON_4_UP as u16, 3, false),
    (RI_MOUSE_BUTTON_5_DOWN as u16, 4, true),
    (RI_MOUSE_BUTTON_5_UP as u16, 4, false),
];

pub struct Shared {
    pub ring: Arc<Ring<Sample>>,
    pub seen: AtomicU64,
    /// Reports delivered while this application was not in the foreground,
    /// which is the evidence that background delivery is working.
    pub background: AtomicU64,
    /// Reports where both wheel bits were set, so the single shared data field
    /// cannot say which axis moved. Counted rather than guessed at.
    pub ambiguous_wheel: AtomicU32,
}

thread_local! {
    static PUMP: Cell<*const Shared> = const { Cell::new(std::ptr::null()) };
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub running: bool,
    pub registered: bool,
    pub error: Option<String>,
}

pub struct RawInputCapture {
    pub ring: Arc<Ring<Sample>>,
    pub shared: Arc<Shared>,
    pub status: Arc<Mutex<Status>>,
    thread_id: u32,
    join: Option<std::thread::JoinHandle<()>>,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_INPUT {
        // Timestamped first: everything after this line is our own latency.
        // Windows attaches no timestamp to a raw input report, so this is the
        // earliest point at which one can exist.
        let t = clock::now();
        let foreground = (wparam as u32 & 0xff) == RIM_INPUT;
        let ctx = PUMP.with(|c| c.get());
        if !ctx.is_null() {
            handle_input(&*ctx, lparam as HRAWINPUT, t, foreground);
        }
        // Documented as required for RIM_INPUT so the system can clean up the
        // kernel's raw input block. Skipping it leaks one per event.
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

unsafe fn handle_input(shared: &Shared, hri: HRAWINPUT, t: u64, foreground: bool) {
    let header_size = size_of::<RAWINPUTHEADER>() as u32;
    // A mouse RAWINPUT is exactly 48 bytes, so the common path is one call
    // rather than the textbook size-then-fetch pair.
    let mut fixed = MaybeUninit::<RAWINPUT>::uninit();
    let mut size = size_of::<RAWINPUT>() as u32;
    let n = GetRawInputData(
        hri,
        RID_INPUT,
        fixed.as_mut_ptr() as *mut c_void,
        &mut size,
        header_size,
    );
    if n != u32::MAX && n != 0 {
        decode(shared, fixed.as_ptr(), t, foreground);
        return;
    }
    if n != u32::MAX {
        return;
    }
    // The failure path must re-probe with a null buffer. Microsoft documents
    // that pcbSize is filled in for GetRawInputDeviceInfoW on failure, but
    // pointedly not for this function, so trusting it here would silently drop
    // every oversized report.
    let mut need: u32 = 0;
    if GetRawInputData(hri, RID_INPUT, null_mut(), &mut need, header_size) != 0 || need == 0 {
        return;
    }
    // Eight-byte aligned: a Vec<u8> is one-aligned and dereferencing a
    // misaligned RAWINPUT is undefined, and faults for real on ARM64.
    let mut buf: Vec<u64> = vec![0; (need as usize + 7) / 8];
    let mut size2 = need;
    let n2 = GetRawInputData(
        hri,
        RID_INPUT,
        buf.as_mut_ptr() as *mut c_void,
        &mut size2,
        header_size,
    );
    if n2 != u32::MAX && n2 != 0 {
        decode(shared, buf.as_ptr() as *const RAWINPUT, t, foreground);
    }
}

unsafe fn decode(shared: &Shared, ri: *const RAWINPUT, t: u64, foreground: bool) {
    if (*ri).header.dwType != RIM_TYPEMOUSE {
        return;
    }
    let m = &(*ri).data.mouse;
    let bits = m.Anonymous.Anonymous;
    let flags = bits.usButtonFlags;
    // Declared unsigned, documented as carrying a signed value. Read as
    // unsigned, a scroll down of 0xFF88 becomes +65416.
    let data = bits.usButtonData as i16 as i32;

    let mut s = Sample {
        t,
        device: (*ri).header.hDevice as usize as u64,
        kind: Kind::Event,
        dx: m.lLastX,
        dy: m.lLastY,
        flags: if foreground {
            0
        } else {
            Flags::BACKGROUND.bits()
        },
        ..Default::default()
    };
    // MOUSE_MOVE_RELATIVE is zero, so this must be a mask test. Comparing for
    // equality misreads a relative packet that also carries NOCOALESCE.
    if m.usFlags & MOUSE_MOVE_ABSOLUTE as u16 != 0 {
        s.dx = 0;
        s.dy = 0;
    }

    for (bit, index, down) in BUTTON_TABLE {
        if flags & bit != 0 {
            if down {
                s.buttons_down |= 1 << index;
            } else {
                s.buttons_up |= 1 << index;
            }
        }
    }

    // Both wheel bits share one data field, so if both are set neither value
    // can be recovered. Count it instead of reporting the same delta twice.
    let vert = flags & RI_MOUSE_WHEEL as u16 != 0;
    let horiz = flags & RI_MOUSE_HWHEEL as u16 != 0;
    if vert && horiz {
        shared.ambiguous_wheel.fetch_add(1, Ordering::Relaxed);
    } else if vert {
        s.wheel = data;
    } else if horiz {
        s.hwheel = data;
    }

    shared.seen.fetch_add(1, Ordering::Relaxed);
    if !foreground {
        shared.background.fetch_add(1, Ordering::Relaxed);
    }
    shared.ring.push(s);
}

fn registrations(hwnd: HWND) -> Vec<RAWINPUTDEVICE> {
    vec![
        // The mouse collection. RIDEV_INPUTSINK is not an optimisation: the
        // default is focus-following, and a message-only window never has
        // focus, so without it nothing arrives at all.
        RAWINPUTDEVICE {
            usUsagePage: 0x01,
            usUsage: 0x02,
            dwFlags: RIDEV_INPUTSINK | RIDEV_DEVNOTIFY,
            hwndTarget: hwnd,
        },
        // Buttons past the fifth cannot come through RAWMOUSE, which has room
        // for exactly five. Gaming mice put them on a vendor or consumer
        // collection instead.
        RAWINPUTDEVICE {
            usUsagePage: 0x0C,
            usUsage: 0,
            dwFlags: RIDEV_INPUTSINK | RIDEV_DEVNOTIFY | RIDEV_PAGEONLY,
            hwndTarget: hwnd,
        },
        RAWINPUTDEVICE {
            usUsagePage: 0xFF00,
            usUsage: 0,
            dwFlags: RIDEV_INPUTSINK | RIDEV_DEVNOTIFY | RIDEV_PAGEONLY,
            hwndTarget: hwnd,
        },
    ]
}

impl RawInputCapture {
    pub fn start(capacity: usize) -> Self {
        let ring: Arc<Ring<Sample>> = Arc::new(Ring::new(capacity));
        let shared = Arc::new(Shared {
            ring: ring.clone(),
            seen: AtomicU64::new(0),
            background: AtomicU64::new(0),
            ambiguous_wheel: AtomicU32::new(0),
        });
        let status = Arc::new(Mutex::new(Status::default()));
        let (tx, rx) = mpsc::channel::<(u32, bool, Option<String>)>();
        let (c_shared, c_status) = (shared.clone(), status.clone());

        let join = std::thread::Builder::new()
            .name("raw-input-pump".into())
            .spawn(move || unsafe { pump(c_shared, c_status, tx) })
            .ok();

        // Block until the pump reports whether it registered, so `start`
        // returns something known rather than merely spawned.
        let (thread_id, registered, error) = rx
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap_or((0, false, Some("raw input pump did not start".into())));
        if let Ok(mut s) = status.lock() {
            s.registered = registered;
            s.running = registered;
            s.error = error;
        }

        RawInputCapture {
            ring,
            shared,
            status,
            thread_id,
            join,
        }
    }

    pub fn take_consumer(&self) -> Option<Consumer> {
        self.ring.take_consumer()
    }

    pub fn status(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn seen(&self) -> u64 {
        self.shared.seen.load(Ordering::Relaxed)
    }

    pub fn background(&self) -> u64 {
        self.shared.background.load(Ordering::Relaxed)
    }

    pub fn stop(&mut self) {
        let join = match self.join.take() {
            Some(j) => j,
            None => return,
        };
        if self.thread_id != 0 {
            // PostQuitMessage cannot be used across threads.
            unsafe { PostThreadMessageW(self.thread_id, WM_STOP_PUMP, 0, 0) };
        }
        let _ = join.join();
        if let Ok(mut s) = self.status.lock() {
            s.running = false;
        }
    }
}

impl Drop for RawInputCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

unsafe fn pump(
    shared: Arc<Shared>,
    _status: Arc<Mutex<Status>>,
    tx: mpsc::Sender<(u32, bool, Option<String>)>,
) {
    // Needs no privilege inside a normal priority class; only
    // REALTIME_PRIORITY_CLASS would, and that is deliberately not used.
    SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);

    let hinst = GetModuleHandleW(null()) as HINSTANCE;
    let class_name = wide("MouseTestingRawInputSink");
    let wc = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style: 0,
        lpfnWndProc: Some(wndproc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinst,
        hIcon: null_mut(),
        hCursor: null_mut(),
        hbrBackground: null_mut(),
        lpszMenuName: null(),
        lpszClassName: class_name.as_ptr(),
        hIconSm: null_mut(),
    };
    if RegisterClassExW(&wc) == 0 {
        let e = std::io::Error::last_os_error();
        // The class outlives the thread, so a restarted capture finds it
        // already registered. That is fine to reuse; anything else is fatal.
        const ERROR_CLASS_ALREADY_EXISTS: i32 = 1410;
        if e.raw_os_error() != Some(ERROR_CLASS_ALREADY_EXISTS) {
            let _ = tx.send((0, false, Some(format!("RegisterClassExW: {e}"))));
            return;
        }
    }

    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        wide("raw input sink").as_ptr(),
        0,
        0,
        0,
        0,
        0,
        HWND_MESSAGE,
        null_mut(),
        hinst,
        null(),
    );
    if hwnd.is_null() {
        let e = std::io::Error::last_os_error();
        let _ = tx.send((0, false, Some(format!("CreateWindowExW: {e}"))));
        return;
    }

    let shared_ref: &Shared = &shared;
    PUMP.with(|c| c.set(shared_ref as *const Shared));

    let regs = registrations(hwnd);
    let ok = RegisterRawInputDevices(
        regs.as_ptr(),
        regs.len() as u32,
        size_of::<RAWINPUTDEVICE>() as u32,
    );
    let registered = ok != 0;
    let err = if registered {
        None
    } else {
        Some(format!(
            "RegisterRawInputDevices: {}",
            std::io::Error::last_os_error()
        ))
    };
    let _ = tx.send((GetCurrentThreadId(), registered, err));

    let mut msg: MSG = zeroed();
    loop {
        let r = GetMessageW(&mut msg, null_mut(), 0, 0);
        if r == 0 || r == -1 {
            break;
        }
        if msg.hwnd.is_null() && msg.message == WM_STOP_PUMP {
            break;
        }
        // No TranslateMessage: there are no characters to cook here and it is
        // pure overhead on the hot path.
        DispatchMessageW(&msg);
    }

    // RIDEV_REMOVE requires a null hwndTarget or the call fails, and the
    // page-wide entries must be removed with RIDEV_PAGEONLY still set or they
    // are not the entries that were registered.
    let removals: Vec<RAWINPUTDEVICE> = regs
        .iter()
        .map(|r| RAWINPUTDEVICE {
            usUsagePage: r.usUsagePage,
            usUsage: r.usUsage,
            dwFlags: RIDEV_REMOVE | (r.dwFlags & RIDEV_PAGEONLY),
            hwndTarget: null_mut(),
        })
        .collect();
    RegisterRawInputDevices(
        removals.as_ptr(),
        removals.len() as u32,
        size_of::<RAWINPUTDEVICE>() as u32,
    );
    PUMP.with(|c| c.set(std::ptr::null()));
    DestroyWindow(hwnd);
    UnregisterClassW(class_name.as_ptr(), hinst);
}

/// Compile-time proof of the layouts the decoder indexes into. If a future
/// Windows SDK changes any of them, this fails to build rather than reading
/// the wrong bytes at runtime.
mod layout {
    use super::*;
    const _: () = assert!(size_of::<RAWINPUTHEADER>() == 24);
    const _: () = assert!(size_of::<RAWINPUT>() == 48);
    const _: () = assert!(size_of::<RAWINPUTDEVICE>() == 16);
    const _: () = assert!(align_of::<RAWINPUT>() == 8);
    use std::mem::align_of;
}
