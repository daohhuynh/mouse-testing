pub mod ab;
pub mod abstats;
pub mod clock;
pub mod cps;
pub mod debounce;
pub mod export;
/// macOS only. IOKit hands over the raw report bytes and the descriptor that
/// says how to read them; Windows raw input arrives already decoded, so there
/// is nothing on that platform for this to parse.
#[cfg(target_os = "macos")]
pub mod hid_descriptor;
pub mod polling;
pub mod ring;
pub mod sensor;
pub mod session_log;
pub mod summary;
pub mod sim;
pub mod sample;
