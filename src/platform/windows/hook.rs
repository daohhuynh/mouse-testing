//! System-level capture on Windows: a low-level mouse hook.
//!
//! The hook fires once per mouse input event, before that event is posted to
//! any thread's input queue, which is where coalescing happens. So this level
//! sees the full report rate while the application level does not, and the
//! difference between them is the thing this program exists to show.
//!
//! One constraint dominates the design. If the hook procedure takes longer
//! than the LowLevelHooksTimeout budget, Windows removes the hook silently and
//! never tells the application. The procedure therefore does nothing but stamp
//! a counter, push plain data into a lock-free ring, and pass the event on; and
//! a watchdog watches for the silence that means it was removed anyway.
//!
//! This level reports a cursor position, not device counts: the position is
//! post-acceleration, post-"enhance pointer precision", quantised to whole
//! screen pixels and clamped at the desktop edge. Distance and counts must come
//! from the device level.

use crate::core::clock;
use crate::core::ring::{Consumer, Ring};
use crate::core::sample::{Flags, Kind, Sample};
use std::cell::Cell;
use std::mem::zeroed;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{
    RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PostThreadMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, HHOOK, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, WH_MOUSE_LL, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
};

const HC_ACTION: i32 = 0;

pub struct Shared {
    pub ring: Arc<Ring<Sample>>,
    pub seen: AtomicU64,
    /// Events Windows marked as injected by software rather than produced by
    /// hardware. A nonzero count means a macro tool, a remote session or a
    /// virtual machine's cursor sync is polluting the measurement.
    pub injected: AtomicU64,
}

thread_local! {
    static HOOK_CTX: Cell<*const Shared> = const { Cell::new(std::ptr::null()) };
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub installed: bool,
    pub error: Option<String>,
    /// The budget, in milliseconds, that the hook procedure must stay inside.
    pub timeout_ms: u32,
    pub timeout_assumed: bool,
}

pub struct HookCapture {
    pub ring: Arc<Ring<Sample>>,
    pub shared: Arc<Shared>,
    pub status: Arc<Mutex<Status>>,
    thread_id: u32,
    join: Option<std::thread::JoinHandle<()>>,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Reads the hook budget. Only a DWORD is accepted: asking for a string as well
/// would let a REG_SZ be copied into a four-byte buffer as raw UTF-16.
fn hooks_timeout_ms(os_build: u32) -> (u32, bool) {
    unsafe {
        let sub = wide("Control Panel\\Desktop");
        let name = wide("LowLevelHooksTimeout");
        let mut value: u32 = 0;
        let mut size: u32 = 4;
        let rc = RegGetValueW(
            HKEY_CURRENT_USER,
            sub.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            &mut value as *mut u32 as *mut _,
            &mut size,
        );
        if rc == 0 && value > 0 {
            (value.min(1000), false)
        } else {
            // Absent on a default install. Windows 10 1709 and later cap the
            // value at 1000 ms; the pre-1709 default is the older 300 ms.
            (if os_build >= 16299 { 1000 } else { 300 }, true)
        }
    }
}

unsafe extern "system" fn ll_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // Stamped before any branch. Everything below is this program's own cost.
    let t = clock::now();
    if code == HC_ACTION {
        let p = lparam as *const MSLLHOOKSTRUCT;
        let ctx = HOOK_CTX.with(|c| c.get());
        if !p.is_null() && !ctx.is_null() {
            let m = *p;
            let shared = &*ctx;
            let msg = wparam as u32;

            let mut s = Sample {
                t,
                device: 0,
                kind: Kind::Event,
                ..Default::default()
            };
            if m.flags & LLMHF_INJECTED != 0 {
                s.flags |= Flags::INJECTED.bits();
                shared.injected.fetch_add(1, Ordering::Relaxed);
            }

            match msg {
                WM_MOUSEMOVE => {
                    // Screen pixels, not device counts. Stored so the level can
                    // report a rate; distance must come from the device level.
                    s.dx = m.pt.x;
                    s.dy = m.pt.y;
                }
                // mouseData's high word is a signed wheel delta.
                WM_MOUSEWHEEL => s.wheel = ((m.mouseData >> 16) as u16 as i16) as i32,
                WM_MOUSEHWHEEL => s.hwheel = ((m.mouseData >> 16) as u16 as i16) as i32,
                WM_LBUTTONDOWN => s.buttons_down = 1 << 0,
                WM_LBUTTONUP => s.buttons_up = 1 << 0,
                WM_RBUTTONDOWN => s.buttons_down = 1 << 1,
                WM_RBUTTONUP => s.buttons_up = 1 << 1,
                WM_MBUTTONDOWN => s.buttons_down = 1 << 2,
                WM_MBUTTONUP => s.buttons_up = 1 << 2,
                WM_XBUTTONDOWN => {
                    let x = (m.mouseData >> 16) as u16;
                    s.buttons_down = 1 << (2 + x.min(2) as u32);
                }
                WM_XBUTTONUP => {
                    let x = (m.mouseData >> 16) as u16;
                    s.buttons_up = 1 << (2 + x.min(2) as u32);
                }
                _ => {}
            }

            shared.seen.fetch_add(1, Ordering::Relaxed);
            shared.ring.push(s);
        }
    }
    // Mandatory. Returning nonzero would swallow the input for the whole
    // desktop; this program is an observer and must never do that.
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

/// Unhooks even if the pump thread unwinds.
struct HookGuard(HHOOK);

impl Drop for HookGuard {
    fn drop(&mut self) {
        unsafe { UnhookWindowsHookEx(self.0) };
    }
}

impl HookCapture {
    pub fn start(capacity: usize, os_build: u32) -> Self {
        let ring: Arc<Ring<Sample>> = Arc::new(Ring::new(capacity));
        let shared = Arc::new(Shared {
            ring: ring.clone(),
            seen: AtomicU64::new(0),
            injected: AtomicU64::new(0),
        });
        let (timeout_ms, timeout_assumed) = hooks_timeout_ms(os_build);
        let status = Arc::new(Mutex::new(Status {
            timeout_ms,
            timeout_assumed,
            ..Default::default()
        }));
        let (tx, rx) = mpsc::channel::<(u32, bool, Option<String>)>();
        let c_shared = shared.clone();

        let join = std::thread::Builder::new()
            .name("mouse-ll-hook".into())
            .spawn(move || unsafe { pump(c_shared, tx) })
            .ok();

        let (thread_id, installed, error) = rx
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap_or((0, false, Some("hook thread did not start".into())));
        if let Ok(mut s) = status.lock() {
            s.installed = installed;
            s.error = error;
        }

        HookCapture {
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

    pub fn injected(&self) -> u64 {
        self.shared.injected.load(Ordering::Relaxed)
    }

    pub fn stop(&mut self) {
        let join = match self.join.take() {
            Some(j) => j,
            None => return,
        };
        if self.thread_id != 0 {
            unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        }
        let _ = join.join();
        if let Ok(mut s) = self.status.lock() {
            s.installed = false;
        }
    }
}

impl Drop for HookCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

unsafe fn pump(shared: Arc<Shared>, tx: mpsc::Sender<(u32, bool, Option<String>)>) {
    SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);

    let shared_ref: &Shared = &shared;
    HOOK_CTX.with(|c| c.set(shared_ref as *const Shared));

    // Passing a module handle rather than null: the documentation warns the
    // call can fail with a null module and a zero thread id.
    let hmod = GetModuleHandleW(std::ptr::null());
    let h: HHOOK = SetWindowsHookExW(WH_MOUSE_LL, Some(ll_proc), hmod, 0);
    if h.is_null() {
        let e = std::io::Error::last_os_error();
        HOOK_CTX.with(|c| c.set(std::ptr::null()));
        let _ = tx.send((0, false, Some(format!("SetWindowsHookExW: {e}"))));
        return;
    }
    let _guard = HookGuard(h);

    // The hook is called by posting to this thread, so it needs a message loop
    // or the procedure is never invoked at all.
    let mut msg: MSG = zeroed();
    let _ = tx.send((GetCurrentThreadId(), true, None));
    loop {
        let r = GetMessageW(&mut msg, null_mut(), 0, 0);
        if r <= 0 {
            break;
        }
        DispatchMessageW(&msg);
    }
    HOOK_CTX.with(|c| c.set(std::ptr::null()));
}
