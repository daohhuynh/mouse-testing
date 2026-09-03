//! Hand-written IOKit bindings.
//!
//! `io-kit-sys` does cover the HID manager, but not `IOHIDCheckAccess` /
//! `IOHIDRequestAccess`, which are the two calls this program needs most, and
//! it pins an older `mach2`. The surface we need is small enough to declare
//! directly and keeps the ABI under our own control -- which matters, because
//! the per-device removal callback has a different arity from the manager one
//! and getting that wrong reads an uninitialised register.

#![allow(non_upper_case_globals, non_snake_case, non_camel_case_types, dead_code)]

use core_foundation_sys::array::CFArrayRef;
use core_foundation_sys::base::{Boolean, CFAllocatorRef, CFIndex, CFTypeRef};
use core_foundation_sys::dictionary::{CFDictionaryRef, CFMutableDictionaryRef};
use core_foundation_sys::set::CFSetRef;
use core_foundation_sys::string::CFStringRef;
use std::ffi::c_void;
use std::os::raw::c_char;

pub enum __IOHIDManager {}
pub enum __IOHIDDevice {}
pub enum __IOHIDElement {}
pub enum __IOHIDValue {}

pub type IOHIDManagerRef = *mut __IOHIDManager;
pub type IOHIDDeviceRef = *mut __IOHIDDevice;
pub type IOHIDElementRef = *mut __IOHIDElement;
pub type IOHIDValueRef = *mut __IOHIDValue;

pub type IOReturn = i32;
pub const kIOReturnSuccess: IOReturn = 0;
/// TCC has not granted Input Monitoring.
pub const kIOReturnNotPermitted: IOReturn = 0xe00002e2u32 as i32;
pub const kIOReturnExclusiveAccess: IOReturn = 0xe00002c7u32 as i32;
pub const kIOReturnNoDevice: IOReturn = 0xe00002c2u32 as i32;

pub const kIOHIDOptionsTypeNone: u32 = 0x00;
/// Seizing a mouse takes it away from the window server, freezing the user's
/// cursor. Declared so it is obvious we never pass it.
pub const kIOHIDOptionsTypeSeizeDevice: u32 = 0x01;

pub const kIOHIDRequestTypePostEvent: u32 = 0;
pub const kIOHIDRequestTypeListenEvent: u32 = 1;

pub const kIOHIDAccessTypeGranted: u32 = 0;
pub const kIOHIDAccessTypeDenied: u32 = 1;
pub const kIOHIDAccessTypeUnknown: u32 = 2;

pub const kHIDPage_GenericDesktop: u32 = 0x01;
pub const kHIDPage_Button: u32 = 0x09;
pub const kHIDUsage_GD_Pointer: u32 = 0x01;
pub const kHIDUsage_GD_Mouse: u32 = 0x02;
pub const kHIDUsage_GD_Keyboard: u32 = 0x06;
pub const kHIDUsage_GD_X: u32 = 0x30;
pub const kHIDUsage_GD_Y: u32 = 0x31;
pub const kHIDUsage_GD_Wheel: u32 = 0x38;

// io_object_t and friends are all mach port names.
pub type io_object_t = u32;
pub type io_iterator_t = io_object_t;
pub type io_registry_entry_t = io_object_t;
pub type io_service_t = io_object_t;
pub type kern_return_t = i32;
pub type mach_port_t = u32;
pub const KERN_SUCCESS: kern_return_t = 0;
/// MACH_PORT_NULL means "the default main port" to every IOKit call here.
pub const kIOMainPortDefault: mach_port_t = 0;

/// Fires once per element decoded out of an input report.
pub type IOHIDValueCallback =
    extern "C" fn(context: *mut c_void, result: IOReturn, sender: *mut c_void, value: IOHIDValueRef);

/// Fires once per physical input report, with the driver's timestamp.
pub type IOHIDReportWithTimeStampCallback = extern "C" fn(
    context: *mut c_void,
    result: IOReturn,
    sender: *mut c_void,
    rtype: u32,
    reportID: u32,
    report: *mut u8,
    reportLength: CFIndex,
    timeStamp: u64,
);

/// Manager-level device arrival/removal.
pub type IOHIDDeviceCallback = extern "C" fn(
    context: *mut c_void,
    result: IOReturn,
    sender: *mut c_void,
    device: IOHIDDeviceRef,
);

/// Per-device removal. Note this is THREE arguments, not four: IOHIDDevice.h
/// declares it as `IOHIDCallback`, which has no trailing device parameter.
/// Declaring it with four reads a register the caller never set.
pub type IOHIDCallback =
    extern "C" fn(context: *mut c_void, result: IOReturn, sender: *mut c_void);

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    pub fn IOHIDManagerCreate(allocator: CFAllocatorRef, options: u32) -> IOHIDManagerRef;
    pub fn IOHIDManagerOpen(m: IOHIDManagerRef, options: u32) -> IOReturn;
    pub fn IOHIDManagerClose(m: IOHIDManagerRef, options: u32) -> IOReturn;
    pub fn IOHIDManagerSetDeviceMatching(m: IOHIDManagerRef, matching: CFDictionaryRef);
    pub fn IOHIDManagerSetDeviceMatchingMultiple(m: IOHIDManagerRef, multiple: CFArrayRef);
    pub fn IOHIDManagerCopyDevices(m: IOHIDManagerRef) -> CFSetRef;
    pub fn IOHIDManagerScheduleWithRunLoop(
        m: IOHIDManagerRef,
        rl: *mut c_void,
        mode: CFStringRef,
    );
    pub fn IOHIDManagerUnscheduleFromRunLoop(
        m: IOHIDManagerRef,
        rl: *mut c_void,
        mode: CFStringRef,
    );
    pub fn IOHIDManagerRegisterDeviceMatchingCallback(
        m: IOHIDManagerRef,
        cb: Option<IOHIDDeviceCallback>,
        ctx: *mut c_void,
    );
    pub fn IOHIDManagerRegisterDeviceRemovalCallback(
        m: IOHIDManagerRef,
        cb: Option<IOHIDDeviceCallback>,
        ctx: *mut c_void,
    );

    pub fn IOHIDDeviceOpen(d: IOHIDDeviceRef, options: u32) -> IOReturn;
    pub fn IOHIDDeviceClose(d: IOHIDDeviceRef, options: u32) -> IOReturn;
    pub fn IOHIDDeviceGetProperty(d: IOHIDDeviceRef, key: CFStringRef) -> CFTypeRef;
    pub fn IOHIDDeviceConformsTo(d: IOHIDDeviceRef, usagePage: u32, usage: u32) -> Boolean;
    pub fn IOHIDDeviceCopyMatchingElements(
        d: IOHIDDeviceRef,
        matching: CFDictionaryRef,
        options: u32,
    ) -> CFArrayRef;
    pub fn IOHIDDeviceScheduleWithRunLoop(d: IOHIDDeviceRef, rl: *mut c_void, mode: CFStringRef);
    pub fn IOHIDDeviceUnscheduleFromRunLoop(d: IOHIDDeviceRef, rl: *mut c_void, mode: CFStringRef);
    pub fn IOHIDDeviceRegisterInputValueCallback(
        d: IOHIDDeviceRef,
        cb: Option<IOHIDValueCallback>,
        ctx: *mut c_void,
    );
    pub fn IOHIDDeviceRegisterInputReportWithTimeStampCallback(
        d: IOHIDDeviceRef,
        report: *mut u8,
        reportLength: CFIndex,
        cb: Option<IOHIDReportWithTimeStampCallback>,
        ctx: *mut c_void,
    );
    pub fn IOHIDDeviceRegisterRemovalCallback(
        d: IOHIDDeviceRef,
        cb: Option<IOHIDCallback>,
        ctx: *mut c_void,
    );

    pub fn IOHIDValueGetElement(v: IOHIDValueRef) -> IOHIDElementRef;
    /// mach_absolute_time ticks, assigned by the driver before the report
    /// reaches us. This is why we can measure report rate at all.
    pub fn IOHIDValueGetTimeStamp(v: IOHIDValueRef) -> u64;
    pub fn IOHIDValueGetIntegerValue(v: IOHIDValueRef) -> CFIndex;
    pub fn IOHIDValueGetLength(v: IOHIDValueRef) -> CFIndex;
    pub fn IOHIDElementGetUsagePage(e: IOHIDElementRef) -> u32;
    pub fn IOHIDElementGetUsage(e: IOHIDElementRef) -> u32;
    pub fn IOHIDElementGetType(e: IOHIDElementRef) -> u32;
    pub fn IOHIDElementGetLogicalMin(e: IOHIDElementRef) -> CFIndex;
    pub fn IOHIDElementGetLogicalMax(e: IOHIDElementRef) -> CFIndex;
    pub fn IOHIDElementIsRelative(e: IOHIDElementRef) -> Boolean;

    // <IOKit/hidsystem/IOHIDLib.h>
    pub fn IOHIDCheckAccess(requestType: u32) -> u32;
    pub fn IOHIDRequestAccess(requestType: u32) -> Boolean;

    // IORegistry, for topology.
    pub fn IOServiceMatching(name: *const c_char) -> CFMutableDictionaryRef;
    pub fn IOServiceGetMatchingServices(
        mainPort: mach_port_t,
        matching: CFMutableDictionaryRef,
        existing: *mut io_iterator_t,
    ) -> kern_return_t;
    pub fn IOIteratorNext(iterator: io_iterator_t) -> io_object_t;
    pub fn IOObjectRelease(object: io_object_t) -> kern_return_t;
    pub fn IOObjectGetClass(object: io_object_t, className: *mut c_char) -> kern_return_t;
    /// Hierarchy-aware, unlike matching on class-name substrings.
    pub fn IOObjectConformsTo(object: io_object_t, className: *const c_char) -> Boolean;
    pub fn IORegistryEntryGetName(entry: io_registry_entry_t, name: *mut c_char) -> kern_return_t;
    pub fn IORegistryEntryGetParentEntry(
        entry: io_registry_entry_t,
        plane: *const c_char,
        parent: *mut io_registry_entry_t,
    ) -> kern_return_t;
    pub fn IORegistryEntryCreateCFProperty(
        entry: io_registry_entry_t,
        key: CFStringRef,
        allocator: CFAllocatorRef,
        options: u32,
    ) -> CFTypeRef;
    pub fn IOHIDDeviceGetService(device: IOHIDDeviceRef) -> io_service_t;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    /// True when this process may observe input events. Unlike
    /// `IOHIDCheckAccess`, this one was stable across runs during testing.
    pub fn CGPreflightListenEventAccess() -> Boolean;
    pub fn CGRequestListenEventAccess() -> Boolean;
    /// System-wide count of delivered events of one type. Needs no permission,
    /// which makes it the floor we can always fall back to.
    pub fn CGEventSourceCounterForEventType(stateID: i32, eventType: u32) -> u32;
    pub fn CGGetEventTapList(
        maxNumberOfTaps: u32,
        tapList: *mut CGEventTapInformation,
        eventTapCount: *mut u32,
    ) -> i32;
}

/// Layout verified against CGEventTypes.h: 48 bytes, fields at
/// 0/4/8/16/24/28/32/36/40/44.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct CGEventTapInformation {
    pub eventTapID: u32,
    pub tapPoint: u32,
    pub options: u32,
    pub eventsOfInterest: u64,
    pub tappingProcess: i32,
    pub processBeingTapped: i32,
    pub enabled: bool,
    pub minUsecLatency: f32,
    pub avgUsecLatency: f32,
    pub maxUsecLatency: f32,
}

pub const kCGEventSourceStateHIDSystemState: i32 = 1;
pub const kCGEventMouseMoved: u32 = 5;
pub const kCGEventLeftMouseDragged: u32 = 6;
pub const kCGEventRightMouseDragged: u32 = 7;
pub const kCGEventOtherMouseDragged: u32 = 27;
/// `kCGEventTapOptionListenOnly` is 1; a tap created with 0 is an ACTIVE tap
/// that sits in the input path and can delay or drop events.
pub const kCGEventTapOptionDefault: u32 = 0;

#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
pub struct MachTimebase {
    pub numer: u32,
    pub denom: u32,
}

extern "C" {
    pub fn mach_timebase_info(info: *mut MachTimebase) -> i32;
    pub fn mach_absolute_time() -> u64;
}

/// Guard for a mach port right obtained from IOKit. Every
/// `IORegistryEntryGetParentEntry` hands back a +1 reference, so a parent walk
/// without this leaks one port per level.
pub struct IoObject(pub io_object_t);

impl Drop for IoObject {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe { IOObjectRelease(self.0) };
        }
    }
}
