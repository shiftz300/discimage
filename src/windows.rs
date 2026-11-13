//! Windows IMAPI2 optical transport and drive discovery.
use crate::{
    codec::COOKED,
    console,
    device::Device,
    probe::transport::Reply,
    recorder::{Recorder, Transport},
};
use anyhow::{Context, Result, ensure};
use std::ffi::c_void;

unsafe extern "C" {
    fn discimage_windows_open(drive: *const u16, output: *mut *mut c_void) -> i32;
    fn discimage_windows_reserve(handle: *mut c_void) -> i32;
    fn discimage_windows_command(
        handle: *mut c_void,
        cdb: *mut u8,
        length: u32,
        data: *mut u8,
        capacity: u32,
        write: bool,
        timeout: u32,
        used: *mut u32,
        status: *mut u8,
        sense: *mut u8,
        times: *mut f64,
    ) -> i32;
    fn discimage_windows_close(handle: *mut c_void);
}

pub(crate) struct Imapi {
    handle: *mut c_void,
}
impl Imapi {
    pub(crate) fn new(id: &str) -> Result<Self> {
        ensure!(
            id.len() == 2 && id.as_bytes()[0].is_ascii_alphabetic() && id.ends_with(':'),
            "expected a Windows optical drive letter"
        );
        let drive = id.encode_utf16().chain([0]).collect::<Vec<_>>();
        let mut handle = std::ptr::null_mut();
        let code = unsafe { discimage_windows_open(drive.as_ptr(), &mut handle) };
        ensure!(
            code >= 0 && !handle.is_null(),
            "cannot open {id} through Windows IMAPI2 (HRESULT 0x{:08x})",
            code as u32
        );
        Ok(Self { handle })
    }
}
impl Drop for Imapi {
    fn drop(&mut self) {
        unsafe { discimage_windows_close(self.handle) };
    }
}

impl Transport for Imapi {
    fn execute(
        &mut self,
        cdb: &[u8],
        bytes: &mut [u8],
        write: bool,
        timeout: u32,
    ) -> Result<Reply> {
        ensure!(
            (6..=16).contains(&cdb.len()) && bytes.len() <= 65536,
            "invalid bounded MMC command"
        );
        let mut command = cdb.to_vec();
        let (mut used, mut status, mut sense) = (0, 0, [0; 32]);
        let mut times = [0.0; 2];
        let trace = std::env::var_os("DISCIMAGE_TRACE_MMC").is_some();
        if trace {
            console::phase(
                "mmc",
                format_args!(
                    "CDB {} | {} bytes | {} | payload {}",
                    crate::hex(cdb),
                    bytes.len(),
                    if write { "out" } else { "in" },
                    if write {
                        crate::hex(&bytes[..bytes.len().min(128)])
                    } else {
                        String::new()
                    }
                ),
            );
        }
        let code = unsafe {
            discimage_windows_command(
                self.handle,
                command.as_mut_ptr(),
                command.len() as u32,
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                write,
                timeout,
                &mut used,
                &mut status,
                sense.as_mut_ptr(),
                times.as_mut_ptr(),
            )
        };
        if trace {
            console::phase(
                "mmc",
                format_args!(
                    "HRESULT {:08x} | status {status:02x} | used {used} | sense {} | data {}",
                    code as u32,
                    crate::hex(&sense),
                    if !write {
                        crate::hex(&bytes[..(used as usize).min(bytes.len()).min(256)])
                    } else {
                        String::new()
                    }
                ),
            );
        }
        ensure!(
            code >= 0,
            "Windows IMAPI2 command {:02x} failed (HRESULT 0x{:08x}, sense {:02x?}); command was not retried",
            cdb[0],
            code as u32,
            sense
        );
        ensure!(
            used as usize <= bytes.len(),
            "invalid IMAPI2 transfer length"
        );
        ensure!(
            times.iter().all(|v| v.is_finite() && *v >= 0.0),
            "invalid native command timestamps"
        );
        Ok(Reply {
            data: bytes[..used as usize].to_vec(),
            status,
            sense,
            end: times[0],
            duration: times[1],
        })
    }
    fn reserve(&mut self) -> Result<()> {
        let code = unsafe { discimage_windows_reserve(self.handle) };
        ensure!(
            code >= 0,
            "cannot reserve the Windows optical drive (IMAPI2 HRESULT 0x{:08x}); close other disc applications",
            code as u32
        );
        Ok(())
    }
}

/// The timing API accepts only the bounded read-only Command enum. Reservation
/// and tray prevention have the same lifetime as macOS's exclusive probe handle.
pub(crate) struct ProbeDevice {
    link: Imapi,
}
impl ProbeDevice {
    pub(crate) fn open(id: &str) -> Result<Self> {
        let mut link = Imapi::new(id)?;
        link.reserve()?;
        if let Err(error) = link
            .execute(&[0x1e, 0, 0, 0, 1, 0], &mut [], false, 10)
            .and_then(Reply::good)
        {
            let _ = link.execute(&[0x1e, 0, 0, 0, 0, 0], &mut [], false, 10);
            return Err(error).context("cannot prevent media removal during CD timing");
        }
        Ok(Self { link })
    }
}
impl crate::probe::transport::Link for ProbeDevice {
    fn command(&mut self, command: crate::probe::transport::Command) -> Result<Reply> {
        let (packet, length, transfer) = command.packet();
        ensure!(
            transfer <= 32 * COOKED,
            "timing transfer exceeds read limit"
        );
        self.link.execute(
            &packet[..usize::from(length)],
            &mut vec![0; transfer],
            false,
            5,
        )
    }
}
impl Drop for ProbeDevice {
    fn drop(&mut self) {
        let _ = self
            .link
            .execute(&[0x1e, 0, 0, 0, 0, 0], &mut [], false, 10);
    }
}

pub(crate) fn devices() -> Result<Vec<Device>> {
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    let mut drives = Vec::new();
    for letter in b'A'..=b'Z' {
        let root = [u16::from(letter), b':' as u16, b'\\' as u16, 0];
        // GetDriveTypeW only examines this NUL-terminated root; no PowerShell/CIM.
        if unsafe { GetDriveTypeW(root.as_ptr()) } == 5 {
            let id = format!("{}:", char::from(letter));
            drives.push(Recorder::new(&id)?.observe().with_context(|| format!("cannot inspect optical drive {id}; close other disc applications and check access"))?);
        }
    }
    Ok(drives)
}

#[cfg(test)]
mod tests;
