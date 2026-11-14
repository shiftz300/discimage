//! SCSI transports. Only the MMC commands constructed by Command are sent.
use crate::mmc::Reply;
use anyhow::Result;

pub(super) use crate::codec::COOKED as SECTOR_BYTES;
pub(super) const READ_CHUNK: u8 = 32;
// Conservative timing read speed, shared by macOS and Windows. Actual spindle
// speed is measured: SET CD SPEED does not guarantee a fixed rotation rate.
pub(super) const READ_SPEED_KBPS: u16 = 1411;

pub(crate) enum Command {
    #[cfg(any(target_os = "macos", test))]
    Inquiry {
        serial: bool,
    },
    #[cfg(any(target_os = "macos", test))]
    DriveSerial,
    Ready,
    Configuration,
    Capacity,
    Toc,
    Disc,
    TrackNumber(#[cfg_attr(not(any(target_os = "macos", windows, test)), allow(dead_code))] u8),
    Speed,
    Flush(u32),
    Read {
        lba: u32,
        count: u8,
        fua: bool,
        read12: bool,
    },
}

impl Command {
    #[cfg(any(target_os = "macos", windows, test))]
    pub(crate) fn packet(self) -> ([u8; 16], u8, usize) {
        let mut bytes = [0; 16];
        let (length, transfer) = match self {
            #[cfg(any(target_os = "macos", test))]
            Self::Inquiry { serial } => {
                bytes[0] = 0x12;
                bytes[1] = u8::from(serial);
                bytes[2] = if serial { 0x80 } else { 0 };
                bytes[4] = if serial { 255 } else { 36 };
                (6, usize::from(bytes[4]))
            }
            #[cfg(any(target_os = "macos", test))]
            Self::DriveSerial => {
                bytes[0] = 0x46;
                bytes[1] = 2;
                bytes[2] = 1;
                bytes[3] = 8;
                bytes[7] = 2;
                (10, 512)
            }
            Self::Ready => (6, 0),
            Self::Configuration => {
                bytes[0] = 0x46;
                bytes[8] = 8;
                (10, 8)
            }
            Self::Capacity => {
                bytes[0] = 0x25;
                (10, 8)
            }
            Self::Toc | Self::Disc | Self::TrackNumber(_) => {
                let (op, length) = match self {
                    Self::Toc => (0x43, 2048u16),
                    Self::Disc => (0x51, 34),
                    _ => (0x52, 48),
                };
                bytes[0] = op;
                if let Self::TrackNumber(number) = self {
                    bytes[1] = 1; // Address is a track number.
                    bytes[5] = number;
                }
                bytes[7..9].copy_from_slice(&length.to_be_bytes());
                (10, usize::from(length))
            }
            Self::Speed => {
                bytes[0] = 0xbb;
                bytes[2..4].copy_from_slice(&READ_SPEED_KBPS.to_be_bytes());
                bytes[4..6].fill(0xff);
                (12, 0)
            }
            Self::Flush(lba) => {
                return Self::Read {
                    lba,
                    count: 0,
                    fua: true,
                    read12: true,
                }
                .packet();
            }
            Self::Read {
                lba,
                count,
                fua,
                read12,
            } => {
                bytes[0] = if read12 { 0xa8 } else { 0x28 };
                bytes[1] = if fua { 8 } else { 0 };
                bytes[2..6].copy_from_slice(&lba.to_be_bytes());
                bytes[if read12 { 9 } else { 8 }] = count;
                (
                    if read12 { 12 } else { 10 },
                    usize::from(count) * SECTOR_BYTES,
                )
            }
        };
        (bytes, length, transfer)
    }
}

pub(crate) trait Link {
    fn command(&mut self, command: Command) -> Result<Reply>;
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use anyhow::{Context, ensure};
    use std::ffi::{CString, c_char, c_void};

    unsafe extern "C" {
        fn discimage_probe_open(
            bsd: *const c_char,
            error: *mut i32,
            exclusive: bool,
        ) -> *mut c_void;
        fn discimage_probe_close(handle: *mut c_void);
        fn discimage_probe_read(
            handle: *mut c_void,
            cdb: *mut u8,
            length: u8,
            data: *mut u8,
            capacity: u32,
            used: *mut u32,
            status: *mut u8,
            sense: *mut u8,
            times: *mut u64,
        ) -> i32;
    }

    pub(in crate::probe) struct Device(*mut c_void);

    impl Device {
        pub fn open(id: &str) -> Result<Self> {
            Self::open_with_access(id, true)
        }
        pub fn for_identity(id: &str) -> Result<Self> {
            Self::open_with_access(id, false)
        }
        fn open_with_access(id: &str, exclusive: bool) -> Result<Self> {
            let native = if id.starts_with("IOService:") {
                id
            } else {
                crate::device::whole_bsd_name(id).context(
                    "expected an optical drive selector or a legacy whole-disc BSD name (diskN)",
                )?
            };
            let name = CString::new(native)?;
            let mut code = 0;
            // The bridge checks the exact registry service (or whole legacy BSD media).
            let handle = unsafe { discimage_probe_open(name.as_ptr(), &mut code, exclusive) };
            ensure!(
                !handle.is_null(),
                "cannot access the selected optical drive (IOKit 0x{:08x}); check discimage devices and unmount only that disc if it is mounted",
                code as u32
            );
            Ok(Self(handle))
        }
    }

    impl Link for Device {
        fn command(&mut self, command: Command) -> Result<Reply> {
            let (mut cdb, length, transfer) = command.packet();
            let mut data = vec![0; transfer];
            let (mut used, mut status, mut sense, mut times) = (0, 0, [0; 32], [0; 2]);
            // Buffers remain valid throughout the synchronous native read.
            let rc = unsafe {
                discimage_probe_read(
                    self.0,
                    cdb.as_mut_ptr(),
                    length,
                    data.as_mut_ptr(),
                    transfer as u32,
                    &mut used,
                    &mut status,
                    sense.as_mut_ptr(),
                    times.as_mut_ptr(),
                )
            };
            ensure!(
                rc == 0,
                "optical command {:02x} failed (IOKit 0x{:08x})",
                cdb[0],
                rc as u32
            );
            ensure!(
                used as usize <= transfer && times[1] >= times[0],
                "invalid optical response"
            );
            data.truncate(used as usize);
            Ok(Reply {
                data,
                status,
                sense,
                end: times[1] as f64 * 1e-9,
                duration: (times[1] - times[0]) as f64 * 1e-9,
            })
        }
    }

    impl Drop for Device {
        fn drop(&mut self) {
            // Device uniquely owns this exclusive-access handle.
            unsafe { discimage_probe_close(self.0) };
        }
    }
}
#[cfg(target_os = "macos")]
pub(super) use mac::Device;

#[cfg(windows)]
mod win {
    /// DiscProbe's Windows scheduling preference, scoped to acquisition. Restore
    /// both settings on success, failure or Ctrl-C, including partial failures.
    pub(crate) struct TimingPriority {
        process: u32,
        thread: i32,
        _same_thread: std::marker::PhantomData<*mut ()>,
    }

    impl TimingPriority {
        pub(crate) fn new() -> Self {
            use windows_sys::Win32::System::Threading::*;
            let mut guard = Self {
                process: 0,
                thread: i32::MAX,
                _same_thread: std::marker::PhantomData,
            };
            // Pseudo-handles belong to this process/thread and need no close.
            unsafe {
                let process = GetCurrentProcess();
                let thread = GetCurrentThread();
                let old_process = GetPriorityClass(process);
                let old_thread = GetThreadPriority(thread);
                if old_process != 0 && SetPriorityClass(process, HIGH_PRIORITY_CLASS) != 0 {
                    guard.process = old_process;
                }
                if old_thread != i32::MAX && SetThreadPriority(thread, THREAD_PRIORITY_HIGHEST) != 0
                {
                    guard.thread = old_thread;
                }
            }
            if guard.process == 0 || guard.thread == i32::MAX {
                crate::console::warning(format_args!(
                    "Windows could not raise all timing priorities; keeping available scheduling settings"
                ));
            }
            guard
        }
    }

    impl Drop for TimingPriority {
        fn drop(&mut self) {
            use windows_sys::Win32::System::Threading::*;
            unsafe {
                if self.thread != i32::MAX {
                    SetThreadPriority(GetCurrentThread(), self.thread);
                }
                if self.process != 0 {
                    SetPriorityClass(GetCurrentProcess(), self.process);
                }
            }
        }
    }
}
#[cfg(windows)]
pub(super) use crate::windows::ProbeDevice as Device;

#[cfg(windows)]
pub(crate) fn timing_priority() -> win::TimingPriority {
    win::TimingPriority::new()
}

#[cfg(not(any(target_os = "macos", windows)))]
pub(super) struct Device;
#[cfg(not(any(target_os = "macos", windows)))]
impl Device {
    pub fn open(_id: &str) -> Result<Self> {
        anyhow::bail!("live CD timing acquisition is available on macOS and Windows")
    }
}
#[cfg(not(any(target_os = "macos", windows)))]
impl Link for Device {
    fn command(&mut self, command: Command) -> Result<Reply> {
        match command {
            Command::Read {
                lba,
                count,
                fua,
                read12,
            } => {
                let _ = (lba, count, fua, read12);
            }
            Command::Flush(lba) => {
                let _ = lba;
            }
            _ => {}
        }
        anyhow::bail!("live CD timing acquisition is unavailable on this platform")
    }
}
