//! Device enumeration through Raw Input.
//!
//! Raw Input needs no privilege, and the metadata strings come from opening the
//! HID interface with `dwDesiredAccess = 0`. That zero is load-bearing: asking
//! for `GENERIC_READ` on a mouse collection always fails with
//! ERROR_ACCESS_DENIED, elevated or not, because the OS class driver holds
//! every mouse and keyboard collection exclusively. With no access rights at
//! all you still get a handle the `HidD_Get*` routines will serve.

use crate::platform::{DeviceInfo, Link, Topology};
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Devices::HumanInterfaceDevice::{
    HidD_GetAttributes, HidD_GetManufacturerString, HidD_GetProductString,
    HidD_GetSerialNumberString, HIDD_ATTRIBUTES,
};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::UI::Input::{
    GetRawInputDeviceInfoW, GetRawInputDeviceList, RAWINPUTDEVICELIST, RIDI_DEVICEINFO,
    RIDI_DEVICENAME, RID_DEVICE_INFO, RIM_TYPEHID, RIM_TYPEMOUSE,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// `\\?\HID#VID_046D&PID_C08B&MI_01&Col01#7&1f2c3d4e&0&0000#{guid}`
pub fn parse_interface_name(name: &str) -> (Option<u16>, Option<u16>, Option<u8>, Option<u8>) {
    // Case is inconsistent between vendors and OS versions.
    let up = name.to_ascii_uppercase();
    let hex4 = |tag: &str| -> Option<u16> {
        let i = up.find(tag)? + tag.len();
        u16::from_str_radix(up.get(i..i + 4)?, 16).ok()
    };
    let hex2 = |tag: &str| -> Option<u8> {
        let i = up.find(tag)? + tag.len();
        u8::from_str_radix(up.get(i..i + 2)?, 16).ok()
    };
    (hex4("VID_"), hex4("PID_"), hex2("MI_"), hex2("&COL"))
}

/// Interface path to device instance id: strip the `\\?\` prefix and the
/// trailing interface GUID, then turn `#` into `\`.
pub fn instance_id_from_interface(name: &str) -> Option<String> {
    let s = name.strip_prefix("\\\\?\\").or_else(|| name.strip_prefix("\\\\.\\"))?;
    let s = match s.rfind("#{") {
        Some(i) => &s[..i],
        None => s,
    };
    Some(s.replace('#', "\\"))
}

unsafe fn device_name(h: HANDLE) -> Option<String> {
    // For RIDI_DEVICENAME, and only for it, pcbSize counts CHARACTERS. Treating
    // it as bytes yields a plausible-looking half-length path.
    let mut chars: u32 = 0;
    GetRawInputDeviceInfoW(h, RIDI_DEVICENAME, null_mut(), &mut chars);
    if chars == 0 || chars > 8192 {
        return None;
    }
    let mut buf = vec![0u16; chars as usize + 1];
    let mut c2 = chars + 1;
    let got = GetRawInputDeviceInfoW(h, RIDI_DEVICENAME, buf.as_mut_ptr() as *mut c_void, &mut c2);
    if got == u32::MAX {
        return None;
    }
    Some(from_wide(&buf))
}

unsafe fn device_info(h: HANDLE) -> Option<RID_DEVICE_INFO> {
    let mut info: RID_DEVICE_INFO = zeroed();
    // The caller must fill cbSize before the call.
    info.cbSize = size_of::<RID_DEVICE_INFO>() as u32;
    let mut bytes = info.cbSize;
    let got = GetRawInputDeviceInfoW(
        h,
        RIDI_DEVICEINFO,
        &mut info as *mut _ as *mut c_void,
        &mut bytes,
    );
    if got == u32::MAX {
        None
    } else {
        Some(info)
    }
}

struct HidHandle(HANDLE);

impl Drop for HidHandle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(self.0) };
        }
    }
}

unsafe fn open_for_metadata(interface: &str) -> Option<HidHandle> {
    let path = wide(interface);
    let h = CreateFileW(
        path.as_ptr(),
        // Zero access rights. Any read right is refused on a mouse collection.
        0,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        null(),
        OPEN_EXISTING,
        FILE_ATTRIBUTE_NORMAL,
        null_mut(),
    );
    if h == INVALID_HANDLE_VALUE || h.is_null() {
        None
    } else {
        Some(HidHandle(h))
    }
}

unsafe fn hid_string(
    h: HANDLE,
    f: unsafe extern "system" fn(HANDLE, *mut c_void, u32) -> bool,
) -> Option<String> {
    // 126 wide chars plus NUL is the HID spec maximum for a string descriptor.
    let mut buf = [0u16; 128];
    if f(h, buf.as_mut_ptr() as *mut c_void, (buf.len() * 2) as u32) {
        let s = from_wide(&buf);
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    } else {
        None
    }
}

pub fn enumerate() -> Vec<DeviceInfo> {
    unsafe { enumerate_inner() }
}

unsafe fn enumerate_inner() -> Vec<DeviceInfo> {
    let cb = size_of::<RAWINPUTDEVICELIST>() as u32;
    let mut count: u32 = 0;
    if GetRawInputDeviceList(null_mut(), &mut count, cb) == u32::MAX {
        return Vec::new();
    }

    let mut list: Vec<RAWINPUTDEVICELIST> = Vec::new();
    // A device can arrive between the sizing call and the fetch, so retry on
    // ERROR_INSUFFICIENT_BUFFER rather than trusting the first count.
    for _ in 0..4 {
        list.resize(count as usize, zeroed());
        let mut n = count;
        let got = GetRawInputDeviceList(list.as_mut_ptr(), &mut n, cb);
        if got != u32::MAX {
            list.truncate(got as usize);
            break;
        }
        if std::io::Error::last_os_error().raw_os_error() != Some(122) {
            return Vec::new();
        }
        count = n;
    }

    let mut out: Vec<DeviceInfo> = Vec::new();
    for entry in &list {
        if entry.dwType != RIM_TYPEMOUSE && entry.dwType != RIM_TYPEHID {
            continue;
        }
        let path = match device_name(entry.hDevice) {
            Some(p) => p,
            None => continue,
        };
        let info = device_info(entry.hDevice);

        // Only keep HID collections that are actually pointing devices; a
        // gaming mouse also publishes vendor collections we do not want in the
        // device picker.
        let (usage_page, usage) = match (entry.dwType, info) {
            (RIM_TYPEMOUSE, _) => (Some(0x01u16), Some(0x02u16)),
            (RIM_TYPEHID, Some(i)) => {
                let hid = i.Anonymous.hid;
                (Some(hid.usUsagePage), Some(hid.usUsage))
            }
            _ => (None, None),
        };
        if !matches!(
            (usage_page, usage),
            (Some(0x01), Some(0x02)) | (Some(0x01), Some(0x01))
        ) {
            continue;
        }

        let (vid_s, pid_s, mi, col) = parse_interface_name(&path);
        let mut vendor_id = vid_s;
        let mut product_id = pid_s;
        let mut version = None;
        let mut manufacturer = None;
        let mut product = None;
        let mut serial = None;

        if let Some(h) = open_for_metadata(&path) {
            manufacturer = hid_string(h.0, HidD_GetManufacturerString);
            product = hid_string(h.0, HidD_GetProductString);
            serial = hid_string(h.0, HidD_GetSerialNumberString);
            let mut attrs: HIDD_ATTRIBUTES = zeroed();
            attrs.Size = size_of::<HIDD_ATTRIBUTES>() as u32;
            if HidD_GetAttributes(h.0, &mut attrs) {
                vendor_id = Some(attrs.VendorID);
                product_id = Some(attrs.ProductID);
                version = Some(attrs.VersionNumber);
            }
        }

        let (buttons, hwheel) = match (entry.dwType, info) {
            (RIM_TYPEMOUSE, Some(i)) => {
                let m = i.Anonymous.mouse;
                (Some(m.dwNumberOfButtons), Some(m.fHasHorizontalWheel != 0))
            }
            _ => (None, None),
        };

        let instance = instance_id_from_interface(&path);
        let topology = instance
            .as_deref()
            .map(super::topology::describe)
            .unwrap_or_else(|| Topology {
                link: Link::Unknown,
                chain: Vec::new(),
            });

        let name = product
            .clone()
            .or_else(|| manufacturer.clone())
            .unwrap_or_else(|| match (vendor_id, product_id) {
                (Some(v), Some(p)) => format!("HID mouse {v:04X}:{p:04X}"),
                _ => "Unnamed pointing device".to_string(),
            });

        let mut extra: Vec<(String, String)> = Vec::new();
        if let Some(m) = mi {
            extra.push(("composite interface".into(), format!("MI_{m:02}")));
        }
        if let Some(c) = col {
            extra.push(("collection".into(), format!("Col{c:02}")));
        }
        if let Some(b) = buttons {
            extra.push((
                "buttons via raw input".into(),
                format!(
                    "{b} (the class driver's count; a mouse with more exposes them on \
                     a separate collection)"
                ),
            ));
        }
        if let (RIM_TYPEMOUSE, Some(i)) = (entry.dwType, info) {
            let m = i.Anonymous.mouse;
            // Deliberately not surfaced as a polling rate: dwSampleRate is a
            // PS/2 concept and reads 0 or a fabricated constant for USB HID.
            extra.push((
                "dwSampleRate".into(),
                format!("{} (PS/2 field, not a polling rate)", m.dwSampleRate),
            ));
        }

        let key = instance.clone().unwrap_or_else(|| path.clone());

        out.push(DeviceInfo {
            key,
            name,
            manufacturer,
            product,
            serial,
            vendor_id,
            product_id,
            version,
            usage_page,
            usage,
            // Windows exposes no advertised interval for a HID mouse at all.
            advertised_interval_us: None,
            advertised_interval_trusted: false,
            buttons_reported: buttons,
            has_horizontal_wheel: hwheel,
            transport: Some(super::topology::transport_of(instance.as_deref())),
            topology,
            raw_path: Some(path),
            streamable: true,
            not_streamable_reason: None,
            extra,
        });
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}
