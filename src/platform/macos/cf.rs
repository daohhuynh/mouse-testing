//! Small Core Foundation helpers.
//!
//! `CFCopyDescription` on a CFNumber produces
//! `<CFNumber 0x..>{value = +1452, type = kCFNumberSInt32Type}`, which is
//! useless, so every read here dispatches on `CFGetTypeID` first.

#![allow(dead_code)]

use core_foundation::base::TCFType;
use core_foundation::string::CFString;
use core_foundation_sys::base::{CFGetTypeID, CFTypeRef};
use core_foundation_sys::data::{CFDataGetBytePtr, CFDataGetLength, CFDataGetTypeID, CFDataRef};
use core_foundation_sys::number::{
    kCFNumberSInt64Type, CFBooleanGetTypeID, CFBooleanGetValue, CFBooleanRef, CFNumberGetTypeID,
    CFNumberGetValue, CFNumberRef,
};
use core_foundation_sys::string::{CFStringGetTypeID, CFStringRef};
use std::ffi::c_void;

/// GET-rule read: the caller does not own the result.
pub unsafe fn as_i64(t: CFTypeRef) -> Option<i64> {
    if t.is_null() || CFGetTypeID(t) != CFNumberGetTypeID() {
        return None;
    }
    let mut v: i64 = 0;
    if CFNumberGetValue(
        t as CFNumberRef,
        kCFNumberSInt64Type,
        &mut v as *mut i64 as *mut c_void,
    ) {
        Some(v)
    } else {
        None
    }
}

pub unsafe fn as_string(t: CFTypeRef) -> Option<String> {
    if t.is_null() || CFGetTypeID(t) != CFStringGetTypeID() {
        return None;
    }
    Some(CFString::wrap_under_get_rule(t as CFStringRef).to_string())
}

pub unsafe fn as_bool(t: CFTypeRef) -> Option<bool> {
    if t.is_null() || CFGetTypeID(t) != CFBooleanGetTypeID() {
        return None;
    }
    Some(CFBooleanGetValue(t as CFBooleanRef))
}

pub unsafe fn as_bytes(t: CFTypeRef) -> Option<Vec<u8>> {
    if t.is_null() || CFGetTypeID(t) != CFDataGetTypeID() {
        return None;
    }
    let d = t as CFDataRef;
    let len = CFDataGetLength(d) as usize;
    let ptr = CFDataGetBytePtr(d);
    if ptr.is_null() {
        return None;
    }
    Some(std::slice::from_raw_parts(ptr, len).to_vec())
}

/// Renders any CF value as short display text.
pub unsafe fn describe(t: CFTypeRef) -> Option<String> {
    if t.is_null() {
        return None;
    }
    if let Some(s) = as_string(t) {
        return Some(s);
    }
    if let Some(v) = as_i64(t) {
        return Some(v.to_string());
    }
    if let Some(b) = as_bool(t) {
        return Some(b.to_string());
    }
    if let Some(b) = as_bytes(t) {
        return Some(format!("{} bytes", b.len()));
    }
    None
}
