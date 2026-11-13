//! Shared MMC response handling and CD recording commands.
use anyhow::{Result, ensure};

#[cfg(any(target_os = "macos", windows, test))]
mod recording;
#[cfg(any(target_os = "macos", windows, test))]
pub(crate) use recording::*;

pub(crate) struct Reply {
    pub data: Vec<u8>,
    pub status: u8,
    pub sense: [u8; 32],
    pub end: f64,
    pub duration: f64,
}

impl Reply {
    pub(crate) fn asc(&self) -> (u8, u8) {
        match self.sense[0] & 0x7f {
            0x72 | 0x73 => (self.sense[2], self.sense[3]),
            _ => (self.sense[12], self.sense[13]),
        }
    }
    pub fn key(&self) -> u8 {
        match self.sense[0] & 0x7f {
            0x70 | 0x71 => self.sense[2] & 15,
            0x72 | 0x73 => self.sense[1] & 15,
            _ => 0,
        }
    }

    pub fn good(self) -> Result<Self> {
        let (asc, ascq) = self.asc();
        let detail = if self.key() == 3 && (asc, ascq) == (0x73, 3) {
            "; drive reported a power calibration area error"
        } else {
            ""
        };
        ensure!(
            self.status == 0,
            "SCSI command failed (status {:02x}, sense {:02x}/{asc:02x}/{ascq:02x}){detail}; check the disc and drive",
            self.status,
            self.key()
        );
        Ok(self)
    }
}
