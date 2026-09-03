//! Fallback so the crate still builds on platforms we do not support. Every
//! capability reports Unsupported rather than returning empty data.

use super::{AccessItem, AccessReport, Availability, DeviceInfo, HostEnv, Tier, TimerFacts};

pub fn enumerate() -> Vec<DeviceInfo> {
    Vec::new()
}

pub fn access_report() -> AccessReport {
    AccessReport {
        items: Tier::ALL
            .iter()
            .map(|&t| AccessItem {
                tier: Some(t),
                name: format!("{} tier", t.short()),
                state: Availability::Unsupported,
                detail: "This platform has no supported capture backend."
                    .to_string(),
                remedy: None,
                remedy_link: None,
            })
            .collect(),
    }
}

pub fn host_env(_devices: &[DeviceInfo], _selected: Option<&str>) -> HostEnv {
    HostEnv {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        timer: TimerFacts {
            name: "std::time::Instant".into(),
            ..Default::default()
        },
        ..Default::default()
    }
}
