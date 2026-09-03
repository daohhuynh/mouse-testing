//! USB topology by walking the PnP device tree with cfgmgr32.
//!
//! All of this is unprivileged. Opening a hub to ask it for the negotiated
//! port speed is not, so speed is reported only when the device tree happens to
//! carry it, and left unknown otherwise rather than guessed.

use crate::platform::{Link, Topology};
use std::ptr::null_mut;
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_Device_IDW, CM_Get_Device_ID_Size, CM_Get_Parent, CM_Locate_DevNodeW,
    CM_LOCATE_DEVNODE_NORMAL, CR_SUCCESS,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn device_id(devinst: u32) -> Option<String> {
    let mut len: u32 = 0;
    if CM_Get_Device_ID_Size(&mut len, devinst, 0) != CR_SUCCESS || len == 0 {
        return None;
    }
    let mut buf = vec![0u16; len as usize + 1];
    if CM_Get_Device_IDW(devinst, buf.as_mut_ptr(), len + 1, 0) != CR_SUCCESS {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..end]))
}

/// Parent chain from the device outward, bounded so a malformed tree cannot
/// spin forever.
pub fn chain(instance_id: &str) -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let id = wide(instance_id);
        let mut devinst: u32 = 0;
        if CM_Locate_DevNodeW(&mut devinst, id.as_ptr(), CM_LOCATE_DEVNODE_NORMAL) != CR_SUCCESS {
            return out;
        }
        let mut cur = devinst;
        for _ in 0..24 {
            match device_id(cur) {
                Some(s) => out.push(s),
                None => break,
            }
            let mut parent: u32 = 0;
            if CM_Get_Parent(&mut parent, cur, 0) != CR_SUCCESS {
                break;
            }
            cur = parent;
        }
    }
    let _ = null_mut::<u8>();
    out
}

pub fn transport_of(instance_id: Option<&str>) -> String {
    let id = match instance_id {
        Some(i) => i.to_ascii_uppercase(),
        None => return "unknown".into(),
    };
    if id.starts_with("BTHENUM") || id.contains("BTHLE") || id.contains("BTHHFENUM") {
        "Bluetooth".into()
    } else if id.starts_with("HID\\VID_") || id.starts_with("USB\\") {
        "USB or HID".into()
    } else if id.starts_with("ACPI") || id.contains("I2CHID") {
        "internal".into()
    } else {
        "unknown".into()
    }
}

pub fn describe(instance_id: &str) -> Topology {
    let chain = chain(instance_id);
    let upper: Vec<String> = chain.iter().map(|s| s.to_ascii_uppercase()).collect();

    if upper.iter().any(|s| s.starts_with("BTHENUM") || s.contains("BTHLE")) {
        return Topology {
            link: Link::Bluetooth,
            chain,
        };
    }
    if upper.iter().any(|s| s.starts_with("ACPI") || s.contains("I2CHID")) && !upper
        .iter()
        .any(|s| s.starts_with("USB\\"))
    {
        return Topology {
            link: Link::Internal,
            chain,
        };
    }

    // Everything strictly between the device's own node and the root hub that
    // is a USB device, excluding the composite-interface children of the device
    // itself, is an intervening hub.
    let root = upper.iter().position(|s| s.starts_with("USB\\ROOT_HUB"));
    let link = match root {
        Some(root_idx) => {
            let hubs = upper[..root_idx]
                .iter()
                .skip(1)
                .filter(|s| s.starts_with("USB\\VID_") && !s.contains("&MI_"))
                .count();
            Link::Usb {
                hub_depth: Some(hubs as u32),
                speed: None,
            }
        }
        // Without a root hub in the chain there is nothing to count against, so
        // say so rather than reporting a depth we did not establish.
        None => Link::Usb {
            hub_depth: None,
            speed: None,
        },
    };
    Topology { link, chain }
}
