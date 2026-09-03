//! What access the app obtained on Windows.
//!
//! Raw Input is the ceiling here and there is no permission to ask for. The
//! honest statement is that no unprivileged path exists below the class driver:
//! the OS opens every mouse and keyboard top-level collection exclusively, so a
//! `ReadFile` on the mouse collection fails whether or not the process is
//! elevated. Going lower needs a signed kernel filter driver, which is a
//! different product.

use crate::platform::{AccessItem, AccessReport, Availability, Tier};
use std::mem::{size_of, zeroed};
use std::ptr::null_mut;
use windows_sys::Win32::UI::Input::{GetRawInputDeviceList, RAWINPUTDEVICELIST, RIM_TYPEMOUSE};

fn mouse_count() -> Option<usize> {
    unsafe {
        let cb = size_of::<RAWINPUTDEVICELIST>() as u32;
        let mut count: u32 = 0;
        if GetRawInputDeviceList(null_mut(), &mut count, cb) == u32::MAX {
            return None;
        }
        let mut list: Vec<RAWINPUTDEVICELIST> = vec![zeroed(); count as usize];
        let mut n = count;
        let got = GetRawInputDeviceList(list.as_mut_ptr(), &mut n, cb);
        if got == u32::MAX {
            return None;
        }
        list.truncate(got as usize);
        Some(list.iter().filter(|e| e.dwType == RIM_TYPEMOUSE).count())
    }
}

pub fn report() -> AccessReport {
    let mice = mouse_count();

    let (state, detail) = match mice {
        Some(0) => (
            Availability::Unknown,
            "Raw Input enumeration works but reports no mouse. Connect one and refresh."
                .to_string(),
        ),
        Some(n) => (
            Availability::Granted,
            format!(
                "Raw Input enumeration succeeded and reports {n} mouse device(s). \
                 Registration needs no privilege and no permission grant; it is confirmed \
                 when a capture starts."
            ),
        ),
        None => (
            Availability::Denied,
            "GetRawInputDeviceList failed. Without it no device can be identified or \
             measured."
                .to_string(),
        ),
    };

    AccessReport {
        items: vec![
            AccessItem {
                tier: Some(Tier::Device),
                name: "Raw Input, background delivery".into(),
                state,
                detail,
                remedy: None,
                remedy_link: None,
            },
            AccessItem {
                tier: Some(Tier::System),
                name: "Low-level mouse hook".into(),
                state: Availability::Unknown,
                detail: "Confirmed when a capture starts. Needs no privilege, but the hook \
                         is silently removed by the OS if its callback ever overruns the \
                         LowLevelHooksTimeout budget, and Windows gives no notification when \
                         that happens, so the capture watches for it."
                    .into(),
                remedy: None,
                remedy_link: None,
            },
            AccessItem {
                tier: Some(Tier::App),
                name: "Events delivered to this window".into(),
                state: Availability::Granted,
                detail: "Always available. Only counts while the pointer is over this window; \
                         Windows synthesises at most one pending move message per thread \
                         queue, which is exactly the coalescing this tier exists to show."
                    .into(),
                remedy: None,
                remedy_link: None,
            },
            AccessItem {
                tier: None,
                name: "Raw HID reports from the mouse collection".into(),
                state: Availability::Unsupported,
                detail: "Not obtainable. Windows opens every mouse and keyboard top-level \
                         collection exclusively, so a read fails whether or not the process \
                         is elevated. Device metadata is still readable by opening the \
                         interface with no access rights, which is how the identifiers below \
                         were obtained."
                    .into(),
                remedy: None,
                remedy_link: None,
            },
        ],
    }
}
