//! Host facts that decide how much the numbers are worth.

use super::access;
use super::ffi::{mach_absolute_time, mach_timebase_info, MachTimebase};
use crate::platform::{EnvWarning, HostEnv, Link, TimerFacts, WarnLevel};
use std::ffi::{c_void, CString};
use std::ptr;

fn sysctl_raw(name: &str) -> Option<Vec<u8>> {
    let cname = CString::new(name).ok()?;
    let mut len: libc::size_t = 0;
    // The sizing call doubles as an existence check: a missing OID and a zero
    // value are different answers and must not be conflated.
    let rc = unsafe {
        libc::sysctlbyname(cname.as_ptr(), ptr::null_mut(), &mut len, ptr::null_mut(), 0)
    };
    if rc != 0 || len == 0 {
        return None;
    }
    let mut buf = vec![0u8; len];
    let rc = unsafe {
        libc::sysctlbyname(
            cname.as_ptr(),
            buf.as_mut_ptr() as *mut c_void,
            &mut len,
            ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    buf.truncate(len);
    Some(buf)
}

pub fn sysctl_string(name: &str) -> Option<String> {
    let mut b = sysctl_raw(name)?;
    while b.last() == Some(&0) {
        b.pop();
    }
    String::from_utf8(b).ok()
}

pub fn sysctl_int(name: &str) -> Option<i64> {
    let b = sysctl_raw(name)?;
    match b.len() {
        4 => Some(i32::from_ne_bytes(b[..4].try_into().ok()?) as i64),
        8 => Some(i64::from_ne_bytes(b[..8].try_into().ok()?)),
        _ => None,
    }
}

pub fn timebase() -> MachTimebase {
    let mut tb = MachTimebase::default();
    unsafe { mach_timebase_info(&mut tb) };
    if tb.denom == 0 {
        MachTimebase { numer: 1, denom: 1 }
    } else {
        tb
    }
}

fn low_power_mode() -> Option<bool> {
    use objc2_foundation::NSProcessInfo;
    Some(NSProcessInfo::processInfo().isLowPowerModeEnabled())
}

fn thermal_state() -> Option<(i64, &'static str)> {
    use objc2_foundation::NSProcessInfo;
    let s = NSProcessInfo::processInfo().thermalState().0 as i64;
    Some((
        s,
        match s {
            0 => "nominal",
            1 => "fair",
            2 => "serious",
            3 => "critical",
            _ => "unknown",
        },
    ))
}

/// Measures the cost of one timestamp and the smallest gap the clock resolves.
fn measure_clock() -> (f64, f64) {
    // Warm up so the commpage is faulted in and the frequency has settled.
    for _ in 0..50_000 {
        std::hint::black_box(unsafe { mach_absolute_time() });
    }
    let tb = timebase();
    let to_ns = |t: u64| t as f64 * tb.numer as f64 / tb.denom as f64;

    let n = 200_000u64;
    let t0 = unsafe { mach_absolute_time() };
    for _ in 0..n {
        std::hint::black_box(unsafe { mach_absolute_time() });
    }
    let t1 = unsafe { mach_absolute_time() };
    let cost = to_ns(t1 - t0) / n as f64;

    let mut min_gap = u64::MAX;
    let mut prev = unsafe { mach_absolute_time() };
    for _ in 0..200_000 {
        let now = unsafe { mach_absolute_time() };
        let d = now.wrapping_sub(prev);
        if d > 0 && d < min_gap {
            min_gap = d;
        }
        prev = now;
    }
    let res = if min_gap == u64::MAX {
        to_ns(1)
    } else {
        to_ns(min_gap)
    };
    (cost, res)
}

pub fn host_env(devices: &[crate::platform::DeviceInfo], selected: Option<&str>) -> HostEnv {
    let tb = timebase();
    let (cost_ns, resolution_ns) = measure_clock();

    let os_version = sysctl_string("kern.osproductversion").unwrap_or_else(|| "unknown".into());
    let build = sysctl_string("kern.osversion").unwrap_or_default();
    let arch = sysctl_string("hw.machine").unwrap_or_else(|| std::env::consts::ARCH.into());
    let model = sysctl_string("hw.model").unwrap_or_default();
    let cpu = sysctl_string("machdep.cpu.brand_string").unwrap_or_else(|| "unknown".into());

    let ncpu = sysctl_int("hw.ncpu").unwrap_or(0);
    let p_cores = sysctl_int("hw.perflevel0.logicalcpu");
    let e_cores = sysctl_int("hw.perflevel1.logicalcpu");
    let cores = match (p_cores, e_cores) {
        (Some(p), Some(e)) => format!("{ncpu} logical ({p} performance, {e} efficiency)"),
        _ => format!("{ncpu} logical"),
    };

    let translated = sysctl_int("sysctl.proc_translated");
    let vmm = sysctl_int("kern.hv_vmm_present");

    let mut warnings: Vec<EnvWarning> = Vec::new();

    match translated {
        Some(1) => warnings.push(EnvWarning {
            level: WarnLevel::Fail,
            title: "Running under Rosetta 2 translation".into(),
            detail: "This is an x86_64 build being translated on Apple Silicon. \
                     Translation adds variable overhead to every timestamp, so interval \
                     measurements are not valid. Build and run the native arm64 binary."
                .into(),
        }),
        Some(0) | None => {}
        Some(_) => {}
    }

    match vmm {
        Some(1) => warnings.push(EnvWarning {
            level: WarnLevel::Fail,
            title: "Running inside a virtual machine".into(),
            detail: "kern.hv_vmm_present is 1. USB transfers and interrupt delivery are \
                     re-timed by the host, so measured polling rate and interval jitter \
                     describe the hypervisor, not the mouse."
                .into(),
        }),
        Some(0) => {}
        _ => {}
    }

    if let Some(true) = low_power_mode() {
        warnings.push(EnvWarning {
            level: WarnLevel::Warn,
            title: "Low Power Mode is on".into(),
            detail: "macOS reduces clock speeds and relaxes scheduling in Low Power Mode. \
                     Interval tails will be worse than the hardware deserves. Turn it off \
                     in System Settings > Battery for a clean run."
                .into(),
        });
    }

    if let Some((s, name)) = thermal_state() {
        if s >= 1 {
            warnings.push(EnvWarning {
                level: if s >= 2 { WarnLevel::Warn } else { WarnLevel::Info },
                title: format!("Thermal state is {name}"),
                detail: "The system is throttling or about to. Interval tails will be \
                         inflated by scheduling delays that have nothing to do with the mouse."
                    .into(),
            });
        }
    }

    // Another process's active event tap sits in the input path ahead of us.
    for tap in access::foreign_taps() {
        let name = tap
            .process
            .rsplit('/')
            .next()
            .unwrap_or(&tap.process)
            .to_string();
        if tap.active {
            warnings.push(EnvWarning {
                level: WarnLevel::Fail,
                title: format!("{name} is filtering mouse input"),
                detail: format!(
                    "pid {} holds an ACTIVE event tap at tap point {} covering mouse events. \
                     An active tap can delay, drop or rewrite events before any application \
                     sees them, so system and application tier numbers describe that tool as \
                     much as your mouse. Quit it before measuring.",
                    tap.pid, tap.tap_point
                ),
            });
        } else {
            warnings.push(EnvWarning {
                level: WarnLevel::Info,
                title: format!("{name} is observing mouse input"),
                detail: format!(
                    "pid {} holds a listen-only event tap. Listen-only taps cannot alter or \
                     delay input, so this does not invalidate anything; it is listed so the \
                     environment is fully described.",
                    tap.pid
                ),
            });
        }
    }

    // Topology of the device under test.
    if let Some(sel) = selected {
        if let Some(dev) = devices.iter().find(|d| d.key == sel) {
            match &dev.topology.link {
                Link::Usb { hub_depth, speed } => {
                    match hub_depth {
                        Some(0) => {}
                        Some(n) => warnings.push(EnvWarning {
                            level: WarnLevel::Warn,
                            title: format!(
                                "Device is behind {} external USB hub{}",
                                n,
                                if *n == 1 { "" } else { "s" }
                            ),
                            detail: "A hub re-schedules transfers onto its own microframe \
                                     timing and shares bandwidth with everything else plugged \
                                     into it. This shows up as interval jitter and occasional \
                                     late reports. Plug the mouse straight into the machine \
                                     before trusting a polling measurement."
                                .into(),
                        }),
                        None => warnings.push(EnvWarning {
                            level: WarnLevel::Info,
                            title: "USB topology inconclusive".into(),
                            detail: "The parent chain did not reach a host controller, so \
                                     whether this device is behind a hub could not be \
                                     determined."
                                .into(),
                        }),
                    }
                    if let Some(s) = speed {
                        if s.starts_with("low") {
                            warnings.push(EnvWarning {
                                level: WarnLevel::Warn,
                                title: "Device negotiated USB low speed".into(),
                                detail: "Low speed caps the polling interval at 10 ms \
                                         (100 Hz) by specification. A high report rate is \
                                         not achievable on this link."
                                    .into(),
                            });
                        }
                    }
                }
                Link::Bluetooth => warnings.push(EnvWarning {
                    level: WarnLevel::Warn,
                    title: "Device is connected over Bluetooth".into(),
                    detail: "Bluetooth batches reports into connection intervals negotiated \
                             by the radio, and macOS does not expose that interval to \
                             applications. Measured intervals reflect the radio schedule as \
                             much as the sensor."
                        .into(),
                }),
                Link::Virtual => warnings.push(EnvWarning {
                    level: WarnLevel::Fail,
                    title: "Selected device is a software HID device".into(),
                    detail: "Its ancestry is IOHIDResource, meaning it is created in software \
                             rather than by hardware. There is no physical mouse behind these \
                             numbers."
                        .into(),
                }),
                Link::Internal => warnings.push(EnvWarning {
                    level: WarnLevel::Info,
                    title: "Selected device is built in".into(),
                    detail: "Internal trackpads report over an Apple-internal transport, not \
                             USB, and macOS reports a placeholder interval for them."
                        .into(),
                }),
                Link::Unknown => {}
            }
        }
    }

    let mut facts: Vec<(String, String)> = vec![
        ("model".into(), model),
        (
            "mach timebase".into(),
            format!(
                "{}/{} ({:.3} ns per tick)",
                tb.numer,
                tb.denom,
                tb.numer as f64 / tb.denom as f64
            ),
        ),
        (
            "rosetta translation".into(),
            match translated {
                Some(1) => "yes".into(),
                Some(0) => "no".into(),
                _ => "not reported".into(),
            },
        ),
        (
            "virtual machine guest".into(),
            match vmm {
                Some(1) => "yes".into(),
                Some(0) => "no".into(),
                _ => "not reported".into(),
            },
        ),
        (
            "low power mode".into(),
            match low_power_mode() {
                Some(true) => "on".into(),
                Some(false) => "off".into(),
                None => "not reported".into(),
            },
        ),
        (
            "thermal state".into(),
            thermal_state()
                .map(|(_, n)| n.to_string())
                .unwrap_or_else(|| "not reported".into()),
        ),
    ];
    facts.retain(|(_, v)| !v.is_empty());

    HostEnv {
        os: format!("macOS {os_version}"),
        os_build: build,
        arch,
        cpu,
        cores,
        timer: TimerFacts {
            name: "mach_absolute_time".into(),
            resolution_ns,
            cost_ns,
            notes: vec![
                "Device tier timestamps come from the driver, not from this process, so \
                 scheduling delays here cannot distort a measured report interval."
                    .into(),
                "System and application tier timestamps are taken in this process and do \
                 carry scheduler jitter."
                    .into(),
            ],
        },
        warnings,
        facts,
    }
}
