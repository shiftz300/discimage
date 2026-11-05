pub mod art;
pub mod audio;
pub mod author;
pub mod burn;
pub mod calibrate;
pub mod codec;
pub mod device;
pub mod iso;
pub mod layout;
#[cfg(target_os = "macos")]
mod macos;
pub mod preview;
pub mod project;
mod spiral;
pub mod target;
pub mod template;

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
