//! SCSI transports. Only the MMC commands constructed by Command are sent.
use anyhow::{Result, ensure};

pub(super) use crate::codec::COOKED as SECTOR_BYTES;
pub(super) const READ_CHUNK: u8 = 32;
pub(super) const READ_SPEED_KBPS: u16 = 1411;

pub(super) enum Command {
    #[cfg(any(target_os = "macos", windows, test))]
    Inquiry {
        serial: bool,
    },
    #[cfg(any(target_os = "macos", windows, test))]
    DriveSerial,
    Ready,
    Configuration,
    Capacity,
    Toc,
    Disc,
    Track,
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
    pub(super) fn packet(self) -> ([u8; 16], u8, usize) {
        let mut bytes = [0; 16];
        let (length, transfer) = match self {
            #[cfg(any(target_os = "macos", windows, test))]
            Self::Inquiry { serial } => {
                bytes[0] = 0x12;
                bytes[1] = u8::from(serial);
                bytes[2] = if serial { 0x80 } else { 0 };
                bytes[4] = if serial { 255 } else { 36 };
                (6, usize::from(bytes[4]))
            }
            #[cfg(any(target_os = "macos", windows, test))]
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
            Self::Toc | Self::Disc | Self::Track => {
                let (op, length) = match self {
                    Self::Toc => (0x43, 2048u16),
                    Self::Disc => (0x51, 34),
                    _ => (0x52, 48),
                };
                bytes[0] = op;
                if matches!(self, Self::Track) {
                    bytes[1] = 1; // Address is a track number.
                    bytes[5] = 1;
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

pub(super) struct Reply {
    pub data: Vec<u8>,
    pub status: u8,
    pub sense: [u8; 32],
    pub end: f64,
    pub duration: f64,
}

impl Reply {
    pub fn key(&self) -> u8 {
        match self.sense[0] & 0x7f {
            0x70 | 0x71 => self.sense[2] & 15,
            0x72 | 0x73 => self.sense[1] & 15,
            _ => 0,
        }
    }

    pub fn good(self) -> Result<Self> {
        let (asc, ascq) = match self.sense[0] & 0x7f {
            0x72 | 0x73 => (self.sense[2], self.sense[3]),
            _ => (self.sense[12], self.sense[13]),
        };
        ensure!(
            self.status == 0,
            "SCSI read failed (status {:02x}, sense {:02x}/{asc:02x}/{ascq:02x}); check the disc and drive",
            self.status,
            self.key()
        );
        Ok(self)
    }
}

pub(super) trait Link {
    fn command(&mut self, command: Command) -> Result<Reply>;
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use anyhow::Context;
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
    use super::*;
    use anyhow::Context;
    use std::{
        ffi::c_void,
        fs::{File, OpenOptions},
        mem::{offset_of, size_of},
        os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
        ptr,
        time::Instant,
    };
    use windows_sys::Win32::{
        Storage::{
            FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE, GetDriveTypeW},
            IscsiDisc::{
                IOCTL_SCSI_PASS_THROUGH_DIRECT, SCSI_IOCTL_DATA_IN, SCSI_IOCTL_DATA_UNSPECIFIED,
                SCSI_PASS_THROUGH_DIRECT,
            },
        },
        System::{
            IO::DeviceIoControl,
            Memory::{
                MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc, VirtualFree,
            },
        },
    };

    #[repr(C)]
    struct Packet {
        request: SCSI_PASS_THROUGH_DIRECT,
        sense: [u8; 32],
    }

    pub(in crate::probe) struct Device {
        handle: File,
        buffer: *mut c_void,
        epoch: Instant,
    }

    impl Device {
        pub fn open(id: &str) -> Result<Self> {
            let [letter] = id.strip_suffix(':').unwrap_or(id).as_bytes() else {
                anyhow::bail!("expected a Windows optical drive letter such as E:");
            };
            ensure!(
                letter.is_ascii_alphabetic(),
                "expected a Windows optical drive letter"
            );
            let letter = letter.to_ascii_uppercase();
            let root = [u16::from(letter), b':' as u16, b'\\' as u16, 0];
            // GetDriveTypeW reads this NUL-terminated drive root. DRIVE_CDROM is 5.
            ensure!(
                unsafe { GetDriveTypeW(root.as_ptr()) } == 5,
                "{}: is not an optical drive",
                char::from(letter)
            );
            // SPTI requires both access flags, even for data-in-only CDBs.
            let handle = OpenOptions::new()
                .read(true)
                .write(true)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .open(format!(r"\\.\{}:", char::from(letter)))
                .context(
                    "cannot open optical drive for Windows SCSI pass-through; check drive access",
                )?;
            // Direct SPTI requires an adapter-aligned buffer. VirtualAlloc is page-aligned.
            let buffer = unsafe {
                VirtualAlloc(
                    ptr::null(),
                    usize::from(READ_CHUNK) * SECTOR_BYTES,
                    MEM_COMMIT | MEM_RESERVE,
                    PAGE_READWRITE,
                )
            };
            if buffer.is_null() {
                return Err(std::io::Error::last_os_error()).context("cannot allocate SCSI buffer");
            }
            Ok(Self {
                handle,
                buffer,
                epoch: Instant::now(),
            })
        }
    }

    impl Drop for Device {
        fn drop(&mut self) {
            // No asynchronous IO outlives this device.
            unsafe { VirtualFree(self.buffer, 0, MEM_RELEASE) };
        }
    }

    impl Link for Device {
        fn command(&mut self, command: Command) -> Result<Reply> {
            let (cdb, length, transfer) = command.packet();
            ensure!(
                transfer <= usize::from(READ_CHUNK) * SECTOR_BYTES,
                "SCSI transfer too large"
            );
            let mut packet = Packet {
                request: SCSI_PASS_THROUGH_DIRECT {
                    Length: size_of::<SCSI_PASS_THROUGH_DIRECT>() as u16,
                    CdbLength: length,
                    SenseInfoLength: 32,
                    DataIn: if transfer == 0 {
                        SCSI_IOCTL_DATA_UNSPECIFIED
                    } else {
                        SCSI_IOCTL_DATA_IN
                    } as u8,
                    DataTransferLength: transfer as u32,
                    TimeOutValue: 5,
                    DataBuffer: if transfer == 0 {
                        ptr::null_mut()
                    } else {
                        self.buffer
                    },
                    SenseInfoOffset: offset_of!(Packet, sense) as u32,
                    Cdb: cdb,
                    ..Default::default()
                },
                sense: [0; 32],
            };
            let buffer = ptr::from_mut(&mut packet).cast();
            let size = size_of::<Packet>() as u32;
            let mut returned = 0;
            let start = Instant::now();
            // The request and its separate data-in buffer remain valid until IO returns.
            let ok = unsafe {
                DeviceIoControl(
                    self.handle.as_raw_handle(),
                    IOCTL_SCSI_PASS_THROUGH_DIRECT,
                    buffer,
                    size,
                    buffer,
                    size,
                    &mut returned,
                    ptr::null_mut(),
                )
            };
            let end = Instant::now();
            if ok == 0 {
                return Err(std::io::Error::last_os_error())
                    .with_context(|| format!("Windows optical command {:02x} failed", cdb[0]));
            }
            ensure!(
                returned as usize >= size_of::<SCSI_PASS_THROUGH_DIRECT>(),
                "short Windows SCSI response"
            );
            let used = if packet.request.ScsiStatus == 0 {
                packet.request.DataTransferLength as usize
            } else {
                0
            };
            ensure!(used <= transfer, "invalid Windows SCSI transfer length");
            // The successful synchronous request initialized the reported prefix.
            let data =
                unsafe { std::slice::from_raw_parts(self.buffer.cast::<u8>(), used) }.to_vec();
            Ok(Reply {
                data,
                status: packet.request.ScsiStatus,
                sense: packet.sense,
                end: (end - self.epoch).as_secs_f64(),
                duration: (end - start).as_secs_f64(),
            })
        }
    }
}
#[cfg(windows)]
pub(super) use win::Device;

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
