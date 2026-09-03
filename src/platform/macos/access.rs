//! What access the app actually obtained, and how to fix it when it did not.
//!
//! The device tier is probed by really opening a matching device, not by
//! trusting a TCC query: `IOHIDCheckAccess` returned Denied on one run and
//! Unknown on another under identical conditions during testing, whereas the
//! open either works or returns a specific `IOReturn` we can quote.

#![allow(non_upper_case_globals)]

use super::cf;
use super::ffi::*;
use crate::platform::{AccessItem, AccessReport, Availability, Tier};
use core_foundation::array::CFArray;
use core_foundation::base::TCFType;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use core_foundation_sys::base::{kCFAllocatorDefault, CFRelease, CFTypeRef};
use core_foundation_sys::set::{CFSetGetCount, CFSetGetValues};
use std::ffi::c_void;

pub const INPUT_MONITORING_PANE: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent";

pub const INPUT_MONITORING_REMEDY: &str = "\
System Settings > Privacy & Security > Input Monitoring, then switch on the app \
that is running this program and QUIT AND REOPEN it. macOS does not apply the \
grant to an already-running process.

When you launch with `cargo run`, the grant attaches to the terminal or editor \
that started it, not to the binary, and an unsigned binary's identity changes on \
every rebuild. Build the app bundle (see README) to get a stable entry.";

fn ret_name(r: IOReturn) -> String {
    match r {
        kIOReturnSuccess => "kIOReturnSuccess".into(),
        kIOReturnNotPermitted => "kIOReturnNotPermitted".into(),
        kIOReturnExclusiveAccess => "kIOReturnExclusiveAccess".into(),
        kIOReturnNoDevice => "kIOReturnNoDevice".into(),
        other => format!("0x{:08x}", other as u32),
    }
}

/// Result of really trying to open a Generic Desktop Mouse.
pub struct OpenProbe {
    pub attempted: bool,
    pub result: IOReturn,
}

pub fn probe_device_open() -> OpenProbe {
    unsafe { probe_device_open_inner() }
}

unsafe fn probe_device_open_inner() -> OpenProbe {
    let mgr = IOHIDManagerCreate(kCFAllocatorDefault, kIOHIDOptionsTypeNone);
    if mgr.is_null() {
        return OpenProbe {
            attempted: false,
            result: kIOReturnNoDevice,
        };
    }
    let d = CFDictionary::from_CFType_pairs(&[
        (
            CFString::new("DeviceUsagePage"),
            CFNumber::from(kHIDPage_GenericDesktop as i32).as_CFType(),
        ),
        (
            CFString::new("DeviceUsage"),
            CFNumber::from(kHIDUsage_GD_Mouse as i32).as_CFType(),
        ),
    ]);
    let arr = CFArray::from_CFTypes(&[d.as_CFType()]);
    IOHIDManagerSetDeviceMatchingMultiple(mgr, arr.as_concrete_TypeRef());
    let _ = IOHIDManagerOpen(mgr, kIOHIDOptionsTypeNone);

    let set = IOHIDManagerCopyDevices(mgr);
    let n = if set.is_null() {
        0
    } else {
        CFSetGetCount(set) as usize
    };
    let mut raw: Vec<*const c_void> = vec![std::ptr::null(); n];
    if n > 0 {
        CFSetGetValues(set, raw.as_mut_ptr());
    }

    let mut probe = OpenProbe {
        attempted: false,
        result: kIOReturnNoDevice,
    };
    for p in raw {
        let dev = p as IOHIDDeviceRef;
        if dev.is_null() {
            continue;
        }
        probe.attempted = true;
        // Shared open only. Seizing would take the mouse away from the window
        // server and freeze the user's cursor.
        let r = IOHIDDeviceOpen(dev, kIOHIDOptionsTypeNone);
        probe.result = r;
        if r == kIOReturnSuccess {
            IOHIDDeviceClose(dev, kIOHIDOptionsTypeNone);
            break;
        }
    }

    if !set.is_null() {
        CFRelease(set as CFTypeRef);
    }
    IOHIDManagerClose(mgr, kIOHIDOptionsTypeNone);
    CFRelease(mgr as CFTypeRef);
    probe
}

pub fn tcc_check() -> u32 {
    unsafe { IOHIDCheckAccess(kIOHIDRequestTypeListenEvent) }
}

pub fn preflight_listen() -> bool {
    unsafe { CGPreflightListenEventAccess() != 0 }
}

/// Raises the system prompt, once, if macOS has not recorded a decision yet.
/// If a decision already exists this returns immediately and shows nothing,
/// which is why the UI must always offer the Settings path as well.
pub fn request_listen() -> bool {
    unsafe { CGRequestListenEventAccess() != 0 }
}

pub fn report() -> AccessReport {
    let probe = probe_device_open();
    let preflight = preflight_listen();
    let tcc = tcc_check();

    let tcc_text = match tcc {
        kIOHIDAccessTypeGranted => "granted",
        kIOHIDAccessTypeDenied => "denied",
        _ => "unknown",
    };

    let (device_state, device_detail) = if !probe.attempted {
        (
            Availability::Unknown,
            "No Generic Desktop Mouse is attached, so the open could not be tried. \
             Connect a mouse and refresh."
                .to_string(),
        )
    } else if probe.result == kIOReturnSuccess {
        (
            Availability::Granted,
            "IOHIDDeviceOpen succeeded with shared access. Per-device reports with \
             driver-assigned timestamps are available."
                .to_string(),
        )
    } else if probe.result == kIOReturnNotPermitted {
        (
            Availability::Denied,
            format!(
                "IOHIDDeviceOpen returned {}. macOS gates Generic Desktop Mouse and \
                 Keyboard collections behind Input Monitoring. Enumeration, identifiers \
                 and the report descriptor still work; only streaming is blocked.",
                ret_name(probe.result)
            ),
        )
    } else {
        (
            Availability::Denied,
            format!("IOHIDDeviceOpen returned {}.", ret_name(probe.result)),
        )
    };

    let mut items = vec![AccessItem {
        tier: Some(Tier::Device),
        name: "IOKit HID, shared open (no root)".into(),
        state: device_state,
        detail: format!(
            "{device_detail}\nCGPreflightListenEventAccess: {}. IOHIDCheckAccess(listen): {}.",
            preflight, tcc_text
        ),
        remedy: if device_state == Availability::Denied {
            Some(INPUT_MONITORING_REMEDY.to_string())
        } else {
            None
        },
        remedy_link: if device_state == Availability::Denied {
            Some(INPUT_MONITORING_PANE.to_string())
        } else {
            None
        },
    }];

    items.push(AccessItem {
        tier: Some(Tier::System),
        name: "System-wide mouse events".into(),
        state: Availability::Unknown,
        detail: "Confirmed when a capture starts. macOS attributes no physical device \
                 to a system mouse event, so this tier is always the sum of every \
                 pointing device in use, never just the selected one."
            .into(),
        remedy: None,
        remedy_link: None,
    });

    items.push(AccessItem {
        tier: Some(Tier::App),
        name: "Events delivered to this window".into(),
        state: Availability::Granted,
        detail: "Always available. Only counts while this window is focused and the \
                 pointer is inside it; macOS delivers nothing to a background app, not \
                 even device events."
            .into(),
        remedy: None,
        remedy_link: None,
    });

    items.push(AccessItem {
        tier: None,
        name: "Exclusive device access (seize)".into(),
        state: Availability::Unsupported,
        detail: "Deliberately not requested. Seizing the device would take it away from \
                 the window server and freeze the cursor, and on this OS it does not \
                 bypass Input Monitoring anyway."
            .into(),
        remedy: None,
        remedy_link: None,
    });

    items.push(AccessItem {
        tier: None,
        name: "Raw HID below the driver".into(),
        state: Availability::Unsupported,
        detail: "No unprivileged path exists on either platform. What this program calls \
                 the device tier is the rate the OS receives, which is as close to the \
                 wire as a normal application can get."
            .into(),
        remedy: None,
        remedy_link: None,
    });

    AccessReport { items }
}

/// Another process holding an active (non-listen-only) event tap sits in the
/// input path and can delay, drop or rewrite events before we ever see them.
pub struct ForeignTap {
    pub pid: i32,
    pub process: String,
    pub tap_point: u32,
    pub active: bool,
}

extern "C" {
    fn proc_pidpath(pid: i32, buffer: *mut c_void, buffersize: u32) -> i32;
}

fn pid_name(pid: i32) -> String {
    let mut buf = vec![0u8; 4096];
    let n = unsafe { proc_pidpath(pid, buf.as_mut_ptr() as *mut c_void, buf.len() as u32) };
    if n <= 0 {
        return format!("pid {pid}");
    }
    buf.truncate(n as usize);
    String::from_utf8_lossy(&buf).to_string()
}

/// Mouse-related event types, as a mask, so we ignore keyboard-only taps.
const MOUSE_TAP_MASK: u64 = (1 << 1)
    | (1 << 2)
    | (1 << 3)
    | (1 << 4)
    | (1 << 5)
    | (1 << 6)
    | (1 << 7)
    | (1 << 22)
    | (1 << 25)
    | (1 << 26)
    | (1 << 27);

pub fn foreign_taps() -> Vec<ForeignTap> {
    let me = std::process::id() as i32;
    let mut list = vec![CGEventTapInformation::default(); 64];
    let mut count: u32 = 0;
    let err = unsafe { CGGetEventTapList(list.len() as u32, list.as_mut_ptr(), &mut count) };
    if err != 0 {
        return Vec::new();
    }
    list.truncate(count as usize);
    list.into_iter()
        .filter(|t| t.tappingProcess != me && t.enabled)
        .filter(|t| t.eventsOfInterest & MOUSE_TAP_MASK != 0)
        .map(|t| ForeignTap {
            pid: t.tappingProcess,
            process: pid_name(t.tappingProcess),
            tap_point: t.tapPoint,
            active: t.options == kCGEventTapOptionDefault,
        })
        .collect()
}

/// Silences the unused-import warning when `cf` is only used by siblings.
#[allow(unused)]
fn _keep(t: CFTypeRef) -> Option<String> {
    unsafe { cf::describe(t) }
}
