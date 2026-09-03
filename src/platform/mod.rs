//! Platform-independent description of what a backend can tell us about the
//! machine and the devices attached to it.
//!
//! The two supported backends are compiled in exclusively. Every field that a
//! given platform cannot answer is an `Option` or an explicit
//! `Availability::Unsupported`, because reporting a zero we did not measure is
//! the one failure mode this program must never have.

use serde::{Deserialize, Serialize};

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "macos")]
pub use macos as backend;

#[cfg(windows)]
pub mod windows;
#[cfg(windows)]
pub use windows as backend;

#[cfg(not(any(target_os = "macos", windows)))]
pub mod unsupported;
#[cfg(not(any(target_os = "macos", windows)))]
pub use unsupported as backend;

/// The three levels at which this program observes the mouse.
///
/// The whole point of the polling section is that these can disagree, so they
/// are modelled as first-class and never reconciled.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Tier {
    /// Reports as the OS receives them from the device, before any processing
    /// the OS does for applications. Per-device.
    Device,
    /// After the OS has turned reports into system input events, but before
    /// delivery to a particular application. System-wide on both platforms.
    System,
    /// What an ordinary application actually receives, after queue coalescing.
    App,
}

impl Tier {
    /// Referenced by the unsupported-platform backend.
    #[allow(dead_code)]
    pub const ALL: [Tier; 3] = [Tier::Device, Tier::System, Tier::App];

    pub fn short(self) -> &'static str {
        match self {
            Tier::Device => "device",
            Tier::System => "system",
            Tier::App => "app",
        }
    }

    /// What the tier is called on this platform, in the platform's own terms.
    pub fn source_name(self) -> &'static str {
        #[cfg(target_os = "macos")]
        match self {
            Tier::Device => "IOHIDDevice input reports",
            Tier::System => "NSEvent global monitor",
            Tier::App => "events delivered to this window",
        }
        #[cfg(windows)]
        match self {
            Tier::Device => "Raw Input (WM_INPUT)",
            Tier::System => "WH_MOUSE_LL hook",
            Tier::App => "events delivered to this window",
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        match self {
            Tier::Device => "unsupported",
            Tier::System => "unsupported",
            Tier::App => "events delivered to this window",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Availability {
    /// Working, with the access we need.
    Granted,
    /// The OS is refusing, and a user action can fix it.
    Denied,
    /// We asked and the OS has not decided yet.
    Unknown,
    /// Cannot work on this platform at all. Never a failure.
    Unsupported,
}

impl Availability {
    pub fn level(self) -> crate::ui::theme::Level {
        use crate::ui::theme::Level;
        match self {
            Availability::Granted => Level::Pass,
            Availability::Denied => Level::Fail,
            Availability::Unknown => Level::Warn,
            Availability::Unsupported => Level::Off,
        }
    }
}

/// One capability the app tried to obtain, and what happened.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccessItem {
    pub tier: Option<Tier>,
    pub name: String,
    pub state: Availability,
    /// What this access does and does not give us.
    pub detail: String,
    /// Exact steps, when the user can change the answer.
    pub remedy: Option<String>,
    /// A URL or command that opens the right settings pane.
    pub remedy_link: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AccessReport {
    pub items: Vec<AccessItem>,
}

impl AccessReport {
    pub fn tier(&self, t: Tier) -> Option<&AccessItem> {
        self.items.iter().find(|i| i.tier == Some(t))
    }

    pub fn tier_state(&self, t: Tier) -> Availability {
        self.tier(t).map(|i| i.state).unwrap_or(Availability::Unknown)
    }
}

/// How a device is physically attached, as far as the OS will admit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Link {
    /// `hub_depth` counts intermediate hubs between the device and the root
    /// hub. 0 means a port on the host controller.
    Usb {
        hub_depth: Option<u32>,
        speed: Option<String>,
    },
    Bluetooth,
    /// Built into the machine (internal trackpad and friends).
    Internal,
    /// A software-created HID device, not real hardware.
    Virtual,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Topology {
    pub link: Link,
    /// Parent chain from the device outward, for display.
    pub chain: Vec<String>,
}

impl Default for Topology {
    fn default() -> Self {
        Topology {
            link: Link::Unknown,
            chain: Vec::new(),
        }
    }
}

/// A pointing device the OS will tell us about.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Stable within a session; used as the selection key.
    pub key: String,
    /// Best available human name.
    pub name: String,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial: Option<String>,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub version: Option<u16>,
    pub usage_page: Option<u16>,
    pub usage: Option<u16>,
    /// What the device or the OS claims the report interval is, in
    /// microseconds. Advertised, never measured.
    pub advertised_interval_us: Option<u32>,
    /// False when the platform is known to fill this field with a placeholder.
    pub advertised_interval_trusted: bool,
    pub buttons_reported: Option<u32>,
    pub has_horizontal_wheel: Option<bool>,
    pub transport: Option<String>,
    pub topology: Topology,
    /// Platform-native path or identifier, shown verbatim.
    pub raw_path: Option<String>,
    /// Whether the backend can actually stream from this device right now.
    pub streamable: bool,
    /// Why not, when it cannot.
    pub not_streamable_reason: Option<String>,
    /// Anything else worth showing, in display order.
    pub extra: Vec<(String, String)>,
}

impl DeviceInfo {
    pub fn ids(&self) -> String {
        match (self.vendor_id, self.product_id) {
            (Some(v), Some(p)) => format!("{v:04X}:{p:04X}"),
            _ => "not reported".to_string(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum WarnLevel {
    Info,
    Warn,
    Fail,
}

impl WarnLevel {
    pub fn level(self) -> crate::ui::theme::Level {
        use crate::ui::theme::Level;
        match self {
            WarnLevel::Info => Level::Info,
            WarnLevel::Warn => Level::Warn,
            WarnLevel::Fail => Level::Fail,
        }
    }
}

/// Something about this machine that changes how much the numbers are worth.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnvWarning {
    pub level: WarnLevel,
    pub title: String,
    pub detail: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TimerFacts {
    pub name: String,
    /// Nominal tick period of the clock we timestamp with.
    pub resolution_ns: f64,
    /// Measured cost of one timestamp, nanoseconds.
    pub cost_ns: f64,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HostEnv {
    pub os: String,
    pub os_build: String,
    pub arch: String,
    pub cpu: String,
    pub cores: String,
    pub timer: TimerFacts,
    pub warnings: Vec<EnvWarning>,
    /// Everything else, in display order.
    pub facts: Vec<(String, String)>,
}

impl HostEnv {
    pub fn worst(&self) -> Option<WarnLevel> {
        self.warnings.iter().map(|w| w.level).max_by_key(|l| match l {
            WarnLevel::Info => 0,
            WarnLevel::Warn => 1,
            WarnLevel::Fail => 2,
        })
    }
}
