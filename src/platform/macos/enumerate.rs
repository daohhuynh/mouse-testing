//! Device enumeration.
//!
//! Everything here works with no Input Monitoring grant: `IOHIDManagerOpen`
//! returning `kIOReturnNotPermitted` does not stop `IOHIDManagerCopyDevices`
//! or property reads, so the device list is always populated even when
//! streaming is blocked.

use super::cf;
use super::ffi::*;
use crate::platform::{DeviceInfo, Link, Topology};
use core_foundation::array::CFArray;
use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use core_foundation_sys::base::{kCFAllocatorDefault, CFRelease, CFTypeRef};
use core_foundation_sys::set::{CFSetGetCount, CFSetGetValues};
use std::ffi::{c_void, CStr, CString};
use std::os::raw::c_char;

fn matching_dict(page: u32, usage: u32) -> CFDictionary<CFString, CFType> {
    CFDictionary::from_CFType_pairs(&[
        (
            CFString::new("DeviceUsagePage"),
            CFNumber::from(page as i32).as_CFType(),
        ),
        (
            CFString::new("DeviceUsage"),
            CFNumber::from(usage as i32).as_CFType(),
        ),
    ])
}

unsafe fn prop(dev: IOHIDDeviceRef, key: &str) -> CFTypeRef {
    let k = CFString::new(key);
    IOHIDDeviceGetProperty(dev, k.as_concrete_TypeRef())
}

unsafe fn prop_i64(dev: IOHIDDeviceRef, key: &str) -> Option<i64> {
    cf::as_i64(prop(dev, key))
}

unsafe fn prop_string(dev: IOHIDDeviceRef, key: &str) -> Option<String> {
    cf::as_string(prop(dev, key))
}

unsafe fn prop_bool(dev: IOHIDDeviceRef, key: &str) -> Option<bool> {
    cf::as_bool(prop(dev, key))
}

/// io_name_t is char[128] for both of these.
unsafe fn name_field(f: impl FnOnce(*mut c_char) -> kern_return_t) -> Option<String> {
    let mut buf = [0 as c_char; 128];
    if f(buf.as_mut_ptr()) != KERN_SUCCESS {
        return None;
    }
    CStr::from_ptr(buf.as_ptr()).to_str().ok().map(str::to_owned)
}

unsafe fn conforms(obj: io_object_t, class_name: &str) -> bool {
    let c = match CString::new(class_name) {
        Ok(c) => c,
        Err(_) => return false,
    };
    IOObjectConformsTo(obj, c.as_ptr()) != 0
}

unsafe fn registry_i64(entry: io_registry_entry_t, key: &str) -> Option<i64> {
    let k = CFString::new(key);
    let v = IORegistryEntryCreateCFProperty(entry, k.as_concrete_TypeRef(), kCFAllocatorDefault, 0);
    if v.is_null() {
        return None;
    }
    let out = cf::as_i64(v);
    CFRelease(v);
    out
}

fn usb_speed_name(v: i64) -> &'static str {
    match v {
        0 => "low (1.5 Mb/s)",
        1 => "full (12 Mb/s)",
        2 => "high (480 Mb/s)",
        3 => "super (5 Gb/s)",
        4 => "super+ (10 Gb/s)",
        5 => "super+ x2 (20 Gb/s)",
        _ => "unknown",
    }
}

/// Walks the IOService parent chain to work out how the device is attached.
///
/// Hub depth is counted with `IOObjectConformsTo`, not class-name matching:
/// on Apple Silicon the root port is literally named `AppleUSB20XHCIARMPort`,
/// so a substring test for "Hub"/"USB20" reports every directly-connected
/// device as being behind a hub.
unsafe fn topology(dev: IOHIDDeviceRef, transport: Option<&str>) -> Topology {
    let mut chain = Vec::new();
    let service = IOHIDDeviceGetService(dev);
    if service == 0 {
        return Topology {
            link: Link::Unknown,
            chain,
        };
    }

    let mut usb_devices = 0u32;
    let mut speed: Option<String> = None;
    let mut saw_hid_resource = false;
    let mut saw_bluetooth = false;
    let mut reached_controller = false;

    let mut cur: io_registry_entry_t = service;
    let mut owned: Option<IoObject> = None;
    let plane = CString::new("IOService").unwrap();

    for _ in 0..32 {
        let class = name_field(|b| IOObjectGetClass(cur, b)).unwrap_or_default();
        let name = name_field(|b| IORegistryEntryGetName(cur, b)).unwrap_or_default();
        if name.is_empty() || name == class {
            chain.push(class.clone());
        } else {
            chain.push(format!("{class} \"{name}\""));
        }

        if conforms(cur, "IOUSBHostDevice") {
            usb_devices += 1;
            if speed.is_none() {
                let s = registry_i64(cur, "Device Speed").or_else(|| registry_i64(cur, "USBSpeed"));
                speed = s.map(|v| usb_speed_name(v).to_string());
            }
        }
        if conforms(cur, "AppleUSBHostController") {
            reached_controller = true;
        }
        // A software HID device hangs off IOHIDResource, and it will happily
        // claim Transport=USB while being no such thing.
        if class.contains("IOHIDResource") || class.contains("IOHIDUserDevice") {
            saw_hid_resource = true;
        }
        if class.contains("Bluetooth") {
            saw_bluetooth = true;
        }

        let mut parent: io_registry_entry_t = 0;
        if IORegistryEntryGetParentEntry(cur, plane.as_ptr(), &mut parent) != KERN_SUCCESS
            || parent == 0
        {
            break;
        }
        // Dropping the previous guard releases the previous rung's reference.
        owned = Some(IoObject(parent));
        cur = owned.as_ref().unwrap().0;
    }
    drop(owned);

    let link = if saw_hid_resource {
        Link::Virtual
    } else if saw_bluetooth || transport == Some("Bluetooth") {
        Link::Bluetooth
    } else if usb_devices > 0 {
        Link::Usb {
            // One IOUSBHostDevice on the chain means the device itself and
            // nothing between it and a root-hub port.
            hub_depth: if reached_controller {
                Some(usb_devices.saturating_sub(1))
            } else {
                None
            },
            speed,
        }
    } else {
        match transport {
            Some("FIFO") | Some("SPU") | Some("SPI") | Some("SPMI") => Link::Internal,
            _ => Link::Unknown,
        }
    };

    Topology { link, chain }
}

/// Devices whose primary usage is Mouse or Keyboard are the only HID devices
/// macOS gates behind Input Monitoring.
fn is_tcc_gated(usage_page: Option<u16>, usage: Option<u16>) -> bool {
    matches!(
        (usage_page, usage),
        (Some(0x01), Some(0x02)) | (Some(0x01), Some(0x06))
    )
}

pub fn enumerate() -> Vec<DeviceInfo> {
    unsafe { enumerate_inner() }
}

unsafe fn enumerate_inner() -> Vec<DeviceInfo> {
    let mgr = IOHIDManagerCreate(kCFAllocatorDefault, kIOHIDOptionsTypeNone);
    if mgr.is_null() {
        return Vec::new();
    }

    let dicts = vec![
        matching_dict(kHIDPage_GenericDesktop, kHIDUsage_GD_Mouse).as_CFType(),
        matching_dict(kHIDPage_GenericDesktop, kHIDUsage_GD_Pointer).as_CFType(),
    ];
    let arr = CFArray::from_CFTypes(&dicts);
    IOHIDManagerSetDeviceMatchingMultiple(mgr, arr.as_concrete_TypeRef());

    // Deliberately ignored: without Input Monitoring this returns
    // kIOReturnNotPermitted, and enumeration still works.
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

    let mut out: Vec<DeviceInfo> = Vec::new();
    for p in raw {
        let dev = p as IOHIDDeviceRef;
        if dev.is_null() {
            continue;
        }

        let product = prop_string(dev, "Product");
        let manufacturer = prop_string(dev, "Manufacturer");
        let transport = prop_string(dev, "Transport");
        let vendor_id = prop_i64(dev, "VendorID").map(|v| v as u16);
        let product_id = prop_i64(dev, "ProductID").map(|v| v as u16);
        let version = prop_i64(dev, "VersionNumber").map(|v| v as u16);
        let serial = prop_string(dev, "SerialNumber");
        let usage_page = prop_i64(dev, "PrimaryUsagePage").map(|v| v as u16);
        let usage = prop_i64(dev, "PrimaryUsage").map(|v| v as u16);
        let location = prop_i64(dev, "LocationID");
        let interval = prop_i64(dev, "ReportInterval").map(|v| v as u32);
        let max_report = prop_i64(dev, "MaxInputReportSize");
        let built_in = prop_bool(dev, "Built-In");
        let descriptor = cf::as_bytes(prop(dev, "ReportDescriptor"));

        let topo = topology(dev, transport.as_deref());

        // ReportInterval is only a real number for USB. Every internal
        // transport on Apple Silicon reports exactly 8000 us, which is an
        // IOHIDFamily default rather than a measurement.
        let interval_trusted = transport.as_deref() == Some("USB") && interval.is_some();

        let name = product
            .clone()
            .or_else(|| manufacturer.clone())
            .unwrap_or_else(|| "Unnamed pointing device".to_string());

        // Identity that survives re-enumeration within a session. The
        // IOHIDDeviceRef pointer would not, and VID/PID are absent on
        // internal devices.
        let key = format!(
            "{}|{}|{}|{}|{}|{}",
            vendor_id.map(|v| v.to_string()).unwrap_or_default(),
            product_id.map(|v| v.to_string()).unwrap_or_default(),
            location.map(|v| v.to_string()).unwrap_or_default(),
            usage_page.unwrap_or(0),
            usage.unwrap_or(0),
            descriptor.as_ref().map(|d| d.len()).unwrap_or(0),
        );

        let gated = is_tcc_gated(usage_page, usage);

        let mut extra: Vec<(String, String)> = Vec::new();
        if let Some(l) = location {
            extra.push(("location id".into(), format!("0x{l:08X}")));
        }
        if let Some(m) = max_report {
            extra.push(("max input report".into(), format!("{m} bytes")));
        }
        if let Some(b) = built_in {
            extra.push(("built in".into(), b.to_string()));
        }
        if let Some(d) = &descriptor {
            extra.push(("report descriptor".into(), format!("{} bytes", d.len())));
        }

        let info = DeviceInfo {
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
            advertised_interval_us: interval,
            advertised_interval_trusted: interval_trusted,
            buttons_reported: None,
            has_horizontal_wheel: None,
            transport,
            topology: topo,
            raw_path: location.map(|l| format!("IOHIDDevice @ location 0x{l:08X}")),
            streamable: gated,
            not_streamable_reason: if gated {
                None
            } else {
                Some(
                    "This collection is not a Generic Desktop Mouse, so it is not \
                     the pointing interface."
                        .into(),
                )
            },
            extra,
        };

        // The internal trackpad enumerates twice with an identical location,
        // descriptor and usage. Streaming both would double-count every report.
        if let Some(existing) = out.iter_mut().find(|d| d.key == info.key) {
            let n = existing
                .extra
                .iter_mut()
                .find(|(k, _)| k == "duplicate collections");
            match n {
                Some((_, v)) => {
                    let c: u32 = v.split_whitespace().next().and_then(|s| s.parse().ok()).unwrap_or(1);
                    *v = format!("{} collections share this identity", c + 1);
                }
                None => existing.extra.push((
                    "duplicate collections".into(),
                    "2 collections share this identity".into(),
                )),
            }
            continue;
        }
        out.push(info);
    }

    if !set.is_null() {
        CFRelease(set as CFTypeRef);
    }
    IOHIDManagerClose(mgr, kIOHIDOptionsTypeNone);
    CFRelease(mgr as CFTypeRef);

    // Real pointing devices first.
    out.sort_by(|a, b| b.streamable.cmp(&a.streamable).then(a.name.cmp(&b.name)));
    out
}
