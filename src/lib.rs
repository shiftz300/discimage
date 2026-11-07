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
pub mod package;
pub mod phase;
pub mod preview;
pub mod probe;
pub mod project;
mod spiral;
pub mod target;
pub mod template;
pub mod timing;

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
