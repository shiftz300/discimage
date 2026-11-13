pub mod art;
pub mod audio;
pub mod author;
pub mod burn;
pub mod calibrate;
pub mod codec;
pub mod console;
pub mod device;
pub mod iso;
pub mod layout;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "macos", windows, test))]
mod mmc;
pub mod package;
pub mod phase;
pub mod preview;
pub mod probe;
pub mod project;
#[cfg(any(target_os = "macos", windows))]
mod recorder;
mod spiral;
pub mod target;
pub mod template;
pub mod timing;
#[cfg(windows)]
mod windows;

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
