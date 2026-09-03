//! Host facts on Windows.

use crate::platform::{DeviceInfo, EnvWarning, HostEnv, Link, TimerFacts, WarnLevel};
use std::mem::{size_of, zeroed};
use std::ptr::null_mut;
use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows_sys::Win32::System::Registry::{
    RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ,
};
use windows_sys::Win32::System::SystemInformation::{
    GetNativeSystemInfo, OSVERSIONINFOW, SYSTEM_INFO,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, IsWow64Process2};

const IMAGE_FILE_MACHINE_UNKNOWN: u16 = 0;
const IMAGE_FILE_MACHINE_I386: u16 = 0x014c;
const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;
const IMAGE_FILE_MACHINE_ARM64: u16 = 0xAA64;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `RtlGetVersion`, never `GetVersionExW`: since Windows 8.1 the latter is
/// routed through the compatibility shim database and reports 6.2 forever
/// unless the binary carries a manifest listing newer OS GUIDs.
fn os_version() -> (u32, u32, u32) {
    unsafe {
        let mut vi: OSVERSIONINFOW = zeroed();
        vi.dwOSVersionInfoSize = size_of::<OSVERSIONINFOW>() as u32;
        if RtlGetVersion(&mut vi) == 0 {
            (vi.dwMajorVersion, vi.dwMinorVersion, vi.dwBuildNumber)
        } else {
            (0, 0, 0)
        }
    }
}

fn machine_name(m: u16) -> &'static str {
    match m {
        IMAGE_FILE_MACHINE_I386 => "x86",
        IMAGE_FILE_MACHINE_AMD64 => "x64",
        IMAGE_FILE_MACHINE_ARM64 => "arm64",
        IMAGE_FILE_MACHINE_UNKNOWN => "unknown",
        _ => "other",
    }
}

/// (process machine, native machine, emulated?)
fn arch_info() -> (u16, u16, bool) {
    unsafe {
        let mut pm: u16 = 0;
        let mut nm: u16 = 0;
        if IsWow64Process2(GetCurrentProcess(), &mut pm, &mut nm) != 0 {
            let emulated = pm != IMAGE_FILE_MACHINE_UNKNOWN;
            let process = if emulated { pm } else { nm };
            (process, nm, emulated)
        } else {
            let mut si: SYSTEM_INFO = zeroed();
            GetNativeSystemInfo(&mut si);
            (IMAGE_FILE_MACHINE_UNKNOWN, IMAGE_FILE_MACHINE_UNKNOWN, false)
        }
    }
}

fn reg_string(subkey: &str, value: &str) -> Option<String> {
    unsafe {
        let sk = wide(subkey);
        let v = wide(value);
        let mut bytes: u32 = 0;
        if RegGetValueW(
            HKEY_LOCAL_MACHINE,
            sk.as_ptr(),
            v.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            null_mut(),
            &mut bytes,
        ) != 0
            || bytes == 0
        {
            return None;
        }
        let mut buf = vec![0u16; (bytes as usize / 2) + 1];
        let mut b2 = bytes;
        if RegGetValueW(
            HKEY_LOCAL_MACHINE,
            sk.as_ptr(),
            v.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            buf.as_mut_ptr() as *mut _,
            &mut b2,
        ) != 0
        {
            return None;
        }
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..end]))
    }
}

/// Hypervisor presence, and whether it actually means "guest".
#[cfg(target_arch = "x86_64")]
fn hypervisor() -> (bool, Option<String>) {
    // core::arch::x86_64::__cpuid is safe on this target: the feature is
    // guaranteed by the target itself.
    let leaf1 = core::arch::x86_64::__cpuid(1);
    if leaf1.ecx & (1 << 31) == 0 {
        return (false, None);
    }
    let v = core::arch::x86_64::__cpuid(0x4000_0000);
    let mut s = Vec::new();
    for r in [v.ebx, v.ecx, v.edx] {
        s.extend_from_slice(&r.to_le_bytes());
    }
    let vendor = String::from_utf8_lossy(&s).trim_end_matches('\0').to_string();
    (true, Some(vendor))
}

#[cfg(not(target_arch = "x86_64"))]
fn hypervisor() -> (bool, Option<String>) {
    // There is no user-mode CPUID equivalent on Windows on ARM, so the honest
    // answer is that we do not know.
    (false, None)
}

fn qpc_facts() -> (f64, f64) {
    unsafe {
        let mut freq: i64 = 0;
        QueryPerformanceFrequency(&mut freq);
        let freq = if freq == 0 { 10_000_000 } else { freq };
        let resolution_ns = 1e9 / freq as f64;

        let mut t = 0i64;
        for _ in 0..50_000 {
            QueryPerformanceCounter(&mut t);
            std::hint::black_box(t);
        }
        let n = 200_000u64;
        let mut t0 = 0i64;
        QueryPerformanceCounter(&mut t0);
        for _ in 0..n {
            QueryPerformanceCounter(&mut t);
            std::hint::black_box(t);
        }
        let mut t1 = 0i64;
        QueryPerformanceCounter(&mut t1);
        let cost_ns = (t1 - t0) as f64 * resolution_ns / n as f64;
        (cost_ns, resolution_ns)
    }
}

pub fn host_env(devices: &[DeviceInfo], selected: Option<&str>) -> HostEnv {
    let (maj, min, build) = os_version();
    let (pm, nm, emulated) = arch_info();
    let (cost_ns, resolution_ns) = qpc_facts();
    let (hv, hv_vendor) = hypervisor();

    let mut warnings: Vec<EnvWarning> = Vec::new();

    if emulated {
        let level = if pm == IMAGE_FILE_MACHINE_AMD64 && nm == IMAGE_FILE_MACHINE_ARM64 {
            WarnLevel::Fail
        } else {
            WarnLevel::Warn
        };
        warnings.push(EnvWarning {
            level,
            title: format!(
                "This build is {} emulated on {}",
                machine_name(pm),
                machine_name(nm)
            ),
            detail: "Emulated code carries translation-cache-dependent latency spikes of \
                     hundreds of microseconds. An interval histogram taken here measures \
                     the emulator, not the mouse. Use a native build for this machine."
                .into(),
        });
    }

    // A hypervisor bit is set on ordinary bare-metal Windows 11 whenever
    // virtualization-based security is on, so this must not be reported as
    // "you are in a VM" without corroboration.
    if hv {
        let manufacturer = reg_string("HARDWARE\\DESCRIPTION\\System\\BIOS", "SystemManufacturer")
            .unwrap_or_default();
        let product = reg_string("HARDWARE\\DESCRIPTION\\System\\BIOS", "SystemProductName")
            .unwrap_or_default();
        let smbios_says_vm = product.to_ascii_lowercase().contains("virtual")
            || manufacturer.to_ascii_lowercase().contains("vmware")
            || manufacturer.to_ascii_lowercase().contains("qemu")
            || manufacturer.to_ascii_lowercase().contains("innotek")
            || manufacturer.to_ascii_lowercase().contains("parallels");
        let vendor = hv_vendor.clone().unwrap_or_default();
        let third_party = !vendor.is_empty() && vendor != "Microsoft Hv";

        if third_party || smbios_says_vm {
            warnings.push(EnvWarning {
                level: WarnLevel::Fail,
                title: "Running inside a virtual machine".into(),
                detail: format!(
                    "Hypervisor vendor {vendor:?}, firmware reports {manufacturer} {product}. \
                     USB transfers and interrupt delivery are re-timed by the host, so \
                     measured polling rate and interval jitter describe the hypervisor \
                     rather than the mouse."
                ),
            });
        } else {
            warnings.push(EnvWarning {
                level: WarnLevel::Info,
                title: "Hypervisor present (Microsoft Hv)".into(),
                detail: "On Windows 11 this normally means virtualization-based security is \
                         enabled, not that this is a virtual machine. The firmware identifies \
                         real hardware. Timing is usable; expect slightly heavier tails."
                    .into(),
            });
        }
    }

    if let Some(sel) = selected {
        if let Some(dev) = devices.iter().find(|d| d.key == sel) {
            match &dev.topology.link {
                Link::Usb { hub_depth, .. } => match hub_depth {
                    Some(0) => {}
                    Some(n) => warnings.push(EnvWarning {
                        level: WarnLevel::Warn,
                        title: format!(
                            "Device is behind {} external USB hub{}",
                            n,
                            if *n == 1 { "" } else { "s" }
                        ),
                        detail: "A hub re-schedules transfers onto its own microframe timing \
                                 and shares bandwidth with everything else plugged into it, \
                                 which shows up as interval jitter and late reports. Plug the \
                                 mouse straight into the machine before trusting a polling \
                                 measurement."
                            .into(),
                    }),
                    None => warnings.push(EnvWarning {
                        level: WarnLevel::Info,
                        title: "USB topology inconclusive".into(),
                        detail: "The device tree walk did not reach a root hub, so whether \
                                 this device sits behind a hub could not be determined."
                            .into(),
                    }),
                },
                Link::Bluetooth => warnings.push(EnvWarning {
                    level: WarnLevel::Warn,
                    title: "Device is connected over Bluetooth".into(),
                    detail: "Bluetooth batches reports into a radio connection interval that \
                             Windows does not expose to applications. Measured intervals \
                             reflect the radio schedule as much as the sensor."
                        .into(),
                }),
                _ => {}
            }
        }
    }

    let mut facts = vec![
        (
            "windows build".into(),
            format!("{maj}.{min}.{build}"),
        ),
        (
            "process architecture".into(),
            machine_name(if emulated { pm } else { nm }).into(),
        ),
        ("native architecture".into(), machine_name(nm).into()),
        (
            "hypervisor bit".into(),
            match (&hv, &hv_vendor) {
                (true, Some(v)) if !v.is_empty() => format!("set, vendor {v:?}"),
                (true, _) => "set".into(),
                (false, _) => "clear".into(),
            },
        ),
    ];
    if let Some(m) = reg_string("HARDWARE\\DESCRIPTION\\System\\BIOS", "SystemManufacturer") {
        facts.push(("firmware manufacturer".into(), m));
    }
    if let Some(p) = reg_string("HARDWARE\\DESCRIPTION\\System\\BIOS", "SystemProductName") {
        facts.push(("firmware product".into(), p));
    }

    HostEnv {
        os: if maj == 10 && build >= 22000 {
            "Windows 11".to_string()
        } else if maj == 10 {
            "Windows 10".to_string()
        } else {
            format!("Windows {maj}.{min}")
        },
        os_build: format!("build {build}"),
        arch: machine_name(if emulated { pm } else { nm }).into(),
        cpu: reg_string(
            "HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0",
            "ProcessorNameString",
        )
        .unwrap_or_else(|| "unknown".into()),
        cores: format!("{} logical", std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0)),
        timer: TimerFacts {
            name: "QueryPerformanceCounter".into(),
            resolution_ns,
            cost_ns,
            notes: vec![
                "Windows attaches no timestamp to a raw input report, so every tier is \
                 stamped inside this process when the event arrives. Scheduling delay \
                 therefore does enter the measurement; it largely cancels in an interval, \
                 which is a difference, but not in an absolute latency."
                    .into(),
            ],
        },
        warnings,
        facts,
    }
}
