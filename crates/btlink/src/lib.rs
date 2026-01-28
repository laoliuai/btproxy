pub mod link;

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "windows")]
pub mod windows;

pub use link::{BtLink, BtLinkConfig};

#[cfg(target_os = "linux")]
pub use linux::{accept_linux_rfcomm, connect_linux_rfcomm};
#[cfg(target_os = "windows")]
pub use windows::{accept_windows_rfcomm, connect_windows_rfcomm};
