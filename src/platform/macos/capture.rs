//! Device-level capture: HID input reports with driver-assigned timestamps.
//!
//! Two callbacks are registered on each device and joined on the timestamp.
//! The report callback fires exactly once per physical report and carries the
//! driver's `mach_absolute_time`, which makes it the only honest basis for a
//! report rate; the value callback carries fields already decoded by IOKit's
//! own descriptor parser, which saves writing one. Neither alone is enough:
//! the report callback does not say what moved, and the element queue behind
//! the value callback is change-driven, so a repeated value may not re-appear.

use super::ffi::*;
use crate::core::hid_descriptor::{self, Field, ReportMap};
use crate::core::ring::{Consumer, Ring};
use crate::core::sample::{Flags, Sample};
use core_foundation::array::CFArray;
use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use core_foundation_sys::base::{kCFAllocatorDefault, CFRelease, CFTypeRef};
use core_foundation_sys::runloop::{
    kCFRunLoopDefaultMode, CFRunLoopAddSource, CFRunLoopGetCurrent, CFRunLoopRemoveSource,
    CFRunLoopRunInMode, CFRunLoopSourceContext, CFRunLoopSourceCreate, CFRunLoopSourceRef,
    CFRunLoopStop, CFRunLoopWakeUp,
};
use core_foundation_sys::set::{CFSetGetCount, CFSetGetValues};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Which devices a capture should stream from.
#[derive(Clone, Debug)]
pub enum Target {
    /// The device whose enumeration key matches.
    Key(String),
    /// Any HID device that will open. Used by the self-test, because macOS
    /// gates only Mouse and Keyboard collections behind Input Monitoring, so
    /// other devices exercise the whole pipeline with no grant at all.
    AnyOpenable { limit: usize },
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub running: bool,
    /// Devices successfully opened.
    pub opened: usize,
    /// Devices matched but refused, with the reason.
    pub refused: Vec<(String, String)>,
    pub names: Vec<String>,
    pub error: Option<String>,
}

/// Everything needed to decode one device's reports, computed once at open
/// time so the callback only does bit extraction.
struct DeviceMap {
    device: u64,
    uses_ids: bool,
    /// (report id, body length in bytes, fields)
    reports: Vec<(u8, usize, Vec<Field>)>,
}

struct Ctx {
    ring: Arc<Ring<Sample>>,
    /// Counts reports seen, so the UI can tell "no motion" from "not running".
    reports: AtomicU64,
    values: AtomicU64,
    /// Values skipped because the element is a buffer or array, for which the
    /// integer accessor is meaningless.
    values_skipped: AtomicU64,
    /// Reports whose fields were successfully decoded from the descriptor.
    decoded: AtomicU64,
    /// Reports for which no matching descriptor field set was found.
    undecoded: AtomicU64,
    /// Filled before the run loop starts, so the callback only ever reads it.
    maps: std::sync::OnceLock<Box<[DeviceMap]>>,
}

pub struct HidCapture {
    pub ring: Arc<Ring<Sample>>,
    pub status: Arc<Mutex<Status>>,
    stop: Arc<AtomicBool>,
    runloop: Arc<AtomicPtr<c_void>>,
    join: Option<std::thread::JoinHandle<()>>,
    ctx: Arc<Ctx>,
}

extern "C" fn report_cb(
    context: *mut c_void,
    result: IOReturn,
    sender: *mut c_void,
    _rtype: u32,
    report_id: u32,
    report: *mut u8,
    len: core_foundation_sys::base::CFIndex,
    time_stamp: u64,
) {
    if result != kIOReturnSuccess || context.is_null() {
        return;
    }
    let ctx = unsafe { &*(context as *const Ctx) };
    ctx.reports.fetch_add(1, Ordering::Relaxed);

    // Nothing here allocates, locks or formats: at 8 kHz this runs eight
    // thousand times a second and any of those would land in the measurement.
    let mut s = Sample::report(time_stamp, sender as u64, true);

    if let Some(maps) = ctx.maps.get() {
        if let Some(m) = maps.iter().find(|m| m.device == sender as u64) {
            if let Some((_, body_len, fields)) =
                m.reports.iter().find(|(id, _, _)| *id == report_id as u8)
            {
                let bytes = if report.is_null() || len <= 0 {
                    &[][..]
                } else {
                    unsafe { std::slice::from_raw_parts(report, len as usize) }
                };
                // Whether IOKit hands back the report with its id byte
                // prefixed is decided by length, not by guessing: a data byte
                // can coincidentally equal the report id.
                let body = if m.uses_ids && bytes.len() == body_len + 1 {
                    &bytes[1..]
                } else {
                    bytes
                };
                let d = hid_descriptor::decode(fields, body);
                s.dx = d.dx;
                s.dy = d.dy;
                s.wheel = d.wheel;
                s.hwheel = d.hwheel;
                s.buttons_state = d.buttons;
                s.flags |= Flags::DECODED.bits();
                ctx.decoded.fetch_add(1, Ordering::Relaxed);
            } else {
                ctx.undecoded.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    ctx.ring.push(s);
}

extern "C" fn value_cb(
    context: *mut c_void,
    result: IOReturn,
    sender: *mut c_void,
    value: IOHIDValueRef,
) {
    if result != kIOReturnSuccess || value.is_null() || context.is_null() {
        return;
    }
    let ctx = unsafe { &*(context as *const Ctx) };
    unsafe {
        let el = IOHIDValueGetElement(value);
        if el.is_null() {
            return;
        }
        // Buffer and array elements have no meaningful integer value.
        if IOHIDValueGetLength(value) > 8 {
            ctx.values_skipped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let s = Sample::value(
            IOHIDValueGetTimeStamp(value),
            sender as u64,
            IOHIDElementGetUsagePage(el) as u16,
            IOHIDElementGetUsage(el) as u16,
            IOHIDValueGetIntegerValue(value) as i32,
        );
        ctx.values.fetch_add(1, Ordering::Relaxed);
        ctx.ring.push(s);
    }
}

extern "C" fn keepalive_perform(_info: *const c_void) {}

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

unsafe fn prop_string(dev: IOHIDDeviceRef, key: &str) -> Option<String> {
    let k = CFString::new(key);
    super::cf::as_string(IOHIDDeviceGetProperty(dev, k.as_concrete_TypeRef()))
}

unsafe fn prop_i64(dev: IOHIDDeviceRef, key: &str) -> Option<i64> {
    let k = CFString::new(key);
    super::cf::as_i64(IOHIDDeviceGetProperty(dev, k.as_concrete_TypeRef()))
}

/// Rebuilds the enumeration key so a device can be matched by it here.
unsafe fn device_key(dev: IOHIDDeviceRef) -> String {
    let descriptor_len = {
        let k = CFString::new("ReportDescriptor");
        super::cf::as_bytes(IOHIDDeviceGetProperty(dev, k.as_concrete_TypeRef()))
            .map(|d| d.len())
            .unwrap_or(0)
    };
    format!(
        "{}|{}|{}|{}|{}|{}",
        prop_i64(dev, "VendorID").map(|v| (v as u16).to_string()).unwrap_or_default(),
        prop_i64(dev, "ProductID").map(|v| (v as u16).to_string()).unwrap_or_default(),
        prop_i64(dev, "LocationID").map(|v| v.to_string()).unwrap_or_default(),
        prop_i64(dev, "PrimaryUsagePage").unwrap_or(0),
        prop_i64(dev, "PrimaryUsage").unwrap_or(0),
        descriptor_len,
    )
}

fn ret_name(r: IOReturn) -> String {
    match r {
        x if x == kIOReturnSuccess => "success".into(),
        x if x == kIOReturnNotPermitted => {
            "not permitted (Input Monitoring not granted)".into()
        }
        x if x == kIOReturnExclusiveAccess => "another process holds the device".into(),
        other => format!("IOReturn 0x{:08x}", other as u32),
    }
}

impl HidCapture {
    pub fn start(target: Target, ring_capacity: usize) -> Self {
        let ring: Arc<Ring<Sample>> = Arc::new(Ring::new(ring_capacity));
        let ctx = Arc::new(Ctx {
            ring: ring.clone(),
            reports: AtomicU64::new(0),
            values: AtomicU64::new(0),
            values_skipped: AtomicU64::new(0),
            decoded: AtomicU64::new(0),
            undecoded: AtomicU64::new(0),
            maps: std::sync::OnceLock::new(),
        });
        let status = Arc::new(Mutex::new(Status::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let runloop = Arc::new(AtomicPtr::new(std::ptr::null_mut()));

        let (c_ctx, c_status, c_stop, c_rl) =
            (ctx.clone(), status.clone(), stop.clone(), runloop.clone());

        let join = std::thread::Builder::new()
            .name("hid-capture".into())
            .spawn(move || unsafe {
                run_loop_thread(target, c_ctx, c_status, c_stop, c_rl);
            })
            .ok();

        HidCapture {
            ring,
            status,
            stop,
            runloop,
            join,
            ctx,
        }
    }

    pub fn take_consumer(&self) -> Option<Consumer> {
        self.ring.take_consumer()
    }

    pub fn reports_seen(&self) -> u64 {
        self.ctx.reports.load(Ordering::Relaxed)
    }

    pub fn values_seen(&self) -> u64 {
        self.ctx.values.load(Ordering::Relaxed)
    }

    pub fn values_skipped(&self) -> u64 {
        self.ctx.values_skipped.load(Ordering::Relaxed)
    }

    pub fn decoded(&self) -> u64 {
        self.ctx.decoded.load(Ordering::Relaxed)
    }

    pub fn undecoded(&self) -> u64 {
        self.ctx.undecoded.load(Ordering::Relaxed)
    }

    pub fn status(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn stop(&mut self) {
        // Taking the handle makes stopping idempotent: Drop also calls this,
        // and stopping twice would touch a run loop whose thread has gone.
        let join = match self.join.take() {
            Some(j) => j,
            None => return,
        };
        // The flag is set first. CFRunLoopStop on a loop that has not started
        // yet is a no-op and is simply lost, so a bounded run-in-mode loop that
        // re-checks a flag is the only shape without a start/stop race.
        self.stop.store(true, Ordering::SeqCst);
        let p = self.runloop.load(Ordering::SeqCst);
        if !p.is_null() {
            unsafe {
                CFRunLoopStop(p as *mut _);
                CFRunLoopWakeUp(p as *mut _);
            }
        }
        let _ = join.join();
        // Release the reference the capture thread took, now that nothing can
        // use it.
        let p = self.runloop.swap(std::ptr::null_mut(), Ordering::SeqCst);
        if !p.is_null() {
            unsafe { CFRelease(p as CFTypeRef) };
        }
    }
}

impl Drop for HidCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

unsafe fn run_loop_thread(
    target: Target,
    ctx: Arc<Ctx>,
    status: Arc<Mutex<Status>>,
    stop: Arc<AtomicBool>,
    runloop: Arc<AtomicPtr<c_void>>,
) {
    let mgr = IOHIDManagerCreate(kCFAllocatorDefault, kIOHIDOptionsTypeNone);
    if mgr.is_null() {
        if let Ok(mut s) = status.lock() {
            s.error = Some("IOHIDManagerCreate returned null".into());
        }
        return;
    }

    match &target {
        Target::Key(_) => {
            let dicts = vec![
                matching_dict(kHIDPage_GenericDesktop, kHIDUsage_GD_Mouse).as_CFType(),
                matching_dict(kHIDPage_GenericDesktop, kHIDUsage_GD_Pointer).as_CFType(),
            ];
            let arr = CFArray::from_CFTypes(&dicts);
            IOHIDManagerSetDeviceMatchingMultiple(mgr, arr.as_concrete_TypeRef());
        }
        Target::AnyOpenable { .. } => {
            // Null matching dictionary means every HID device.
            IOHIDManagerSetDeviceMatching(mgr, std::ptr::null());
        }
    }

    // Deliberately not fatal: without Input Monitoring this fails, yet
    // enumeration and non-gated devices still work.
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

    // Context handed to C. One allocation, reused for every registration, and
    // reclaimed only after every callback has been set back to None.
    let ctx_ptr = Arc::into_raw(ctx) as *mut c_void;

    let mut opened: Vec<IOHIDDeviceRef> = Vec::new();
    let mut buffers: Vec<(*mut [u8], usize)> = Vec::new();
    let mut refused: Vec<(String, String)> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut seen_keys: Vec<String> = Vec::new();
    let mut maps: Vec<DeviceMap> = Vec::new();

    for p in raw {
        let dev = p as IOHIDDeviceRef;
        if dev.is_null() {
            continue;
        }
        let name = prop_string(dev, "Product").unwrap_or_else(|| "unnamed".into());

        match &target {
            Target::Key(k) => {
                if &device_key(dev) != k {
                    continue;
                }
                // The same collection can be enumerated more than once; opening
                // both would double-count every report.
                let dk = device_key(dev);
                if seen_keys.contains(&dk) {
                    continue;
                }
                seen_keys.push(dk);
            }
            Target::AnyOpenable { limit } => {
                if opened.len() >= *limit {
                    continue;
                }
            }
        }

        let r = IOHIDDeviceOpen(dev, kIOHIDOptionsTypeNone);
        if r != kIOReturnSuccess {
            refused.push((name.clone(), ret_name(r)));
            continue;
        }

        IOHIDDeviceRegisterInputValueCallback(dev, Some(value_cb), ctx_ptr);

        let sz = prop_i64(dev, "MaxInputReportSize").unwrap_or(64).clamp(1, 65536) as usize;
        // Raw allocation rather than a live Box: IOKit writes into this buffer
        // while Rust would otherwise consider it exclusively owned.
        let buf: *mut [u8] = Box::into_raw(vec![0u8; sz].into_boxed_slice());
        IOHIDDeviceRegisterInputReportWithTimeStampCallback(
            dev,
            buf as *mut u8,
            sz as core_foundation_sys::base::CFIndex,
            Some(report_cb),
            ctx_ptr,
        );
        buffers.push((buf, sz));

        // Decoding is driven by the descriptor rather than by IOKit's value
        // callback, which is change-driven and was observed delivering nothing
        // at all for devices that were reporting normally.
        if let Some(desc) = {
            let k = CFString::new("ReportDescriptor");
            super::cf::as_bytes(IOHIDDeviceGetProperty(dev, k.as_concrete_TypeRef()))
        } {
            let map: ReportMap = hid_descriptor::parse(&desc);
            let reports = map
                .report_bits
                .iter()
                .map(|&(id, bits)| {
                    (id, ((bits as usize) + 7) / 8, map.fields_for(id))
                })
                .filter(|(_, _, f)| !f.is_empty())
                .collect::<Vec<_>>();
            maps.push(DeviceMap {
                device: dev as u64,
                uses_ids: map.uses_report_ids,
                reports,
            });
        }

        IOHIDDeviceScheduleWithRunLoop(dev, CFRunLoopGetCurrent() as *mut c_void, kCFRunLoopDefaultMode);
        opened.push(dev);
        names.push(name);
    }

    // Published before any callback can run, and never mutated afterwards.
    let ctx_ref = &*(ctx_ptr as *const Ctx);
    let _ = ctx_ref.maps.set(maps.into_boxed_slice());

    // A run loop with no sources returns from its run call immediately, so a
    // capture that opened nothing would exit before it could be stopped.
    let mut src_ctx = CFRunLoopSourceContext {
        version: 0,
        info: std::ptr::null_mut(),
        retain: None,
        release: None,
        copyDescription: None,
        equal: None,
        hash: None,
        schedule: None,
        cancel: None,
        // Must be built field by field: zeroing this struct is undefined
        // because `perform` is a non-nullable function pointer, and doing it
        // aborts the process rather than failing gracefully.
        perform: keepalive_perform,
    };
    let keepalive: CFRunLoopSourceRef =
        CFRunLoopSourceCreate(kCFAllocatorDefault, 0, &mut src_ctx);
    CFRunLoopAddSource(CFRunLoopGetCurrent(), keepalive, kCFRunLoopDefaultMode);

    if let Ok(mut s) = status.lock() {
        s.running = true;
        s.opened = opened.len();
        s.refused = refused;
        s.names = names;
    }

    // Retain the run loop before publishing it. The controlling thread may
    // call CFRunLoopStop after this thread has already exited, and without an
    // owned reference that would dereference a freed run loop.
    let rl = CFRunLoopGetCurrent();
    core_foundation_sys::base::CFRetain(rl as CFTypeRef);
    runloop.store(rl as *mut c_void, Ordering::SeqCst);

    while !stop.load(Ordering::SeqCst) {
        CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.25, 0);
    }

    // Teardown order matters: every callback must be deregistered before the
    // context or the buffers are freed, or a report arriving during shutdown
    // dereferences freed memory.
    for (i, &dev) in opened.iter().enumerate() {
        IOHIDDeviceUnscheduleFromRunLoop(dev, CFRunLoopGetCurrent() as *mut c_void, kCFRunLoopDefaultMode);
        IOHIDDeviceRegisterInputValueCallback(dev, None, std::ptr::null_mut());
        let (buf, sz) = buffers[i];
        IOHIDDeviceRegisterInputReportWithTimeStampCallback(
            dev,
            buf as *mut u8,
            sz as core_foundation_sys::base::CFIndex,
            None,
            std::ptr::null_mut(),
        );
        IOHIDDeviceClose(dev, kIOHIDOptionsTypeNone);
    }
    CFRunLoopRemoveSource(CFRunLoopGetCurrent(), keepalive, kCFRunLoopDefaultMode);
    CFRelease(keepalive as CFTypeRef);
    IOHIDManagerClose(mgr, kIOHIDOptionsTypeNone);
    if !set.is_null() {
        CFRelease(set as CFTypeRef);
    }
    CFRelease(mgr as CFTypeRef);
    for (buf, _) in buffers {
        drop(Box::from_raw(buf));
    }
    drop(Arc::from_raw(ctx_ptr as *const Ctx));

    if let Ok(mut s) = status.lock() {
        s.running = false;
    }
}

/// Silences the unused warning for a flag only the joiner reads.
#[allow(unused)]
fn _flags(s: &Sample) -> bool {
    s.has(Flags::KERNEL_TIME)
}
