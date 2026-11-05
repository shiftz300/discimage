//! The only unsafe boundary: owned UTF-8 JSON and one opaque Objective-C session.
use crate::device::{self, Device, Music};
use anyhow::{Context, Result, ensure};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::{
    ffi::{CStr, CString, c_char, c_void},
    path::Path,
    ptr,
};

unsafe extern "C" {
    fn discimage_call(
        handle: *mut *mut c_void,
        json: *const c_char,
        cancelled: extern "C" fn() -> i32,
    ) -> *mut c_char;
    fn discimage_free(value: *mut c_char);
}
extern "C" fn cancelled() -> i32 {
    i32::from(device::cancelled())
}

pub struct Recorder {
    handle: *mut c_void,
    id: String,
}
impl Recorder {
    pub fn new(id: &str) -> Self {
        Self {
            handle: ptr::null_mut(),
            id: id.into(),
        }
    }
    fn request<T: DeserializeOwned>(&mut self, value: Value) -> Result<T> {
        let input = CString::new(serde_json::to_vec(&value)?)?;
        // The bridge borrows the request during this call. It retains the opaque
        // session until close, and returns a malloc-owned string, even on errors.
        let output = unsafe { discimage_call(&mut self.handle, input.as_ptr(), cancelled) };
        ensure!(!output.is_null(), "native recorder returned no response");
        let bytes = unsafe { CStr::from_ptr(output).to_bytes().to_vec() };
        unsafe { discimage_free(output) };
        let result: Value =
            serde_json::from_slice(&bytes).context("invalid native recording response")?;
        if let Some(error) = result.get("error").and_then(Value::as_str) {
            anyhow::bail!("{error}");
        }
        serde_json::from_value(result).context("native response schema mismatch")
    }
}
impl Drop for Recorder {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            let _: Result<Value> = self.request(json!({"op":"close"}));
        }
    }
}
impl device::Recorder for Recorder {
    fn inspect(&mut self) -> Result<Device> {
        self.request(if self.handle.is_null() {
            json!({"op":"probe","device":self.id})
        } else {
            json!({"op":"status"})
        })
    }
    fn lock(&mut self) -> Result<()> {
        let _: Value = self.request(json!({"op":"open","device":self.id}))?;
        Ok(())
    }
    fn music(&mut self, tracks: &[Music], speed: u32) -> Result<()> {
        let _: Value = self.request(json!({"op":"audio","tracks":tracks,"speed":speed}))?;
        Ok(())
    }
    fn data(&mut self, path: &Path, start: u32, sectors: u32, speed: u32) -> Result<()> {
        let _: Value = self.request(
            json!({"op":"data","path":path,"start":start,"sectors":sectors,"speed":speed}),
        )?;
        Ok(())
    }
}
pub fn devices() -> Result<Vec<Device>> {
    let value: Value = Recorder::new("").request(json!({"op":"list"}))?;
    Ok(serde_json::from_value(value["devices"].clone())?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_framework_produces_relative_blocks_and_accepts_cd_audio_without_a_drive() {
        let root = std::env::temp_dir().join(format!("discimage-native-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("data.iso");
        let mut bytes = Vec::new();
        for i in 0..4 {
            bytes.extend([i as u8; 2048]);
        }
        std::fs::write(&path, bytes).unwrap();
        let wav = root.join("audio.wav");
        let mut audio = Vec::new();
        let length = 300u32 * 2352;
        audio.extend(b"RIFF");
        audio.extend((length + 36).to_le_bytes());
        audio.extend(b"WAVEfmt ");
        audio.extend(16u32.to_le_bytes());
        audio.extend(1u16.to_le_bytes());
        audio.extend(2u16.to_le_bytes());
        audio.extend(44100u32.to_le_bytes());
        audio.extend(176400u32.to_le_bytes());
        audio.extend(4u16.to_le_bytes());
        audio.extend(16u16.to_le_bytes());
        audio.extend(b"data");
        audio.extend(length.to_le_bytes());
        audio.resize(44 + length as usize, 0);
        std::fs::write(&wav, audio).unwrap();
        let response: Value = Recorder::new("")
            .request(json!({"op":"selftest","path":path,"audio":wav}))
            .unwrap();
        assert_eq!(response["bytes"], 4096);
        assert_eq!(response["first"], 1);
        assert_eq!(response["last"], 2);
        assert_eq!(response["audio_sectors"], 300);
        assert_eq!(response["data_block_size"], 2048);
        std::fs::remove_dir_all(root).unwrap();
    }
}
