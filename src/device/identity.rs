//! Hardware identity uses SCSI vendor/product and a standard drive serial.
//! Prefer MMC feature 0x0108; fall back to SCSI VPD page 0x80.
//! Never substitute OS paths, firmware versions, USB ports or bridge serials.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub vendor: String,
    pub product: String,
    pub serial: String,
}

impl Identity {
    pub(crate) fn from_configuration(inquiry: &[u8], response: &[u8]) -> Option<Self> {
        if inquiry.len() < 36 || inquiry[0] & 31 != 5 {
            return None;
        }
        let identity = Self {
            vendor: String::from_utf8(inquiry[8..16].to_vec()).ok()?,
            product: String::from_utf8(inquiry[16..32].to_vec()).ok()?,
            serial: serial_feature(response)?,
        };
        identity.id()?;
        Some(identity)
    }
    #[cfg(any(target_os = "macos", target_os = "linux", windows, test))]
    pub(crate) fn from_inquiry(inquiry: &[u8], serial: &[u8]) -> Option<Self> {
        if inquiry.len() < 36 || inquiry[0] & 31 != 5 {
            return None;
        }
        let identity = Self {
            vendor: String::from_utf8(inquiry[8..16].to_vec()).ok()?,
            product: String::from_utf8(inquiry[16..32].to_vec()).ok()?,
            serial: serial_page(serial)?,
        };
        identity.id()?;
        Some(identity)
    }
    pub fn id(&self) -> Option<String> {
        fn field(value: &str) -> Option<String> {
            let value = value.trim_matches(|c: char| c == '\0' || c.is_ascii_whitespace());
            (!value.is_empty()
                && value.len() <= 256
                && value.bytes().all(|c| (0x20..=0x7e).contains(&c)))
            .then(|| value.to_string())
        }
        let vendor = field(&self.vendor)?
            .split_ascii_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_uppercase();
        let product = field(&self.product)?
            .split_ascii_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_uppercase();
        let serial = field(&self.serial)?;
        let simple = serial
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_uppercase();
        if serial.len() < 4
            || simple.is_empty()
            || simple.bytes().all(|c| c == b'0')
            || simple.bytes().all(|c| c == b'F')
            || [
                "UNKNOWN",
                "NONE",
                "SERIALNUMBER",
                "DEFAULT",
                "1234567890",
                "0123456789",
                "TOBEFILLEDBYOEM",
            ]
            .contains(&simple.as_str())
        {
            return None;
        }
        let mut hash = Sha256::new();
        hash.update(b"discimage-drive-unit-serial-v1\0");
        for value in [&vendor, &product, &serial] {
            hash.update((value.len() as u32).to_be_bytes());
            hash.update(value.as_bytes());
        }
        Some(crate::hex(&hash.finalize()[..6]))
    }
}

fn serial_feature(response: &[u8]) -> Option<String> {
    if response.len() < 12 {
        return None;
    }
    let declared = u32::from_be_bytes(response[..4].try_into().ok()?) as usize;
    if declared < 8 || declared.checked_add(4)? > response.len() || response[8..10] != [1, 8] {
        return None;
    }
    let count = usize::from(response[11]);
    if count == 0 || 12 + count > declared + 4 {
        return None;
    }
    String::from_utf8(response.get(12..12 + count)?.to_vec()).ok()
}

#[cfg(any(target_os = "macos", target_os = "linux", windows, test))]
fn serial_page(page: &[u8]) -> Option<String> {
    if page.len() < 4 || page[0] & 31 != 5 || page[1] != 0x80 {
        return None;
    }
    let length = usize::from(u16::from_be_bytes([page[2], page[3]]));
    if length == 0 || length > 256 {
        return None;
    }
    String::from_utf8(page.get(4..4 + length)?.to_vec()).ok()
}

#[cfg(target_os = "linux")]
pub(super) fn linux(device: &std::path::Path) -> Option<Identity> {
    use std::io::Read;
    let vendor = std::fs::read_to_string(device.join("vendor")).ok()?;
    let product = std::fs::read_to_string(device.join("model")).ok()?;
    if let Some(value) = linux_mmc(device) {
        return Some(value);
    }
    let mut page = Vec::new();
    std::fs::File::open(device.join("vpd_pg80"))
        .ok()?
        .take(261)
        .read_to_end(&mut page)
        .ok()?;
    Some(Identity {
        vendor,
        product,
        serial: serial_page(&page)?,
    })
}

// SG_IO v3, from Linux include/scsi/sg.h. Only fixed, data-in metadata
// commands are constructed here. No write, speed, media or reset commands.
#[cfg(target_os = "linux")]
fn linux_mmc(device: &std::path::Path) -> Option<Identity> {
    use std::{
        ffi::{c_int, c_void},
        os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    };
    #[repr(C)]
    struct Header {
        interface_id: c_int,
        direction: c_int,
        command_len: u8,
        sense_len: u8,
        iovec_count: u16,
        length: u32,
        data: *mut c_void,
        command: *mut u8,
        sense: *mut u8,
        timeout: u32,
        flags: u32,
        pack_id: c_int,
        user: *mut c_void,
        status: u8,
        masked_status: u8,
        message: u8,
        written_sense: u8,
        host: u16,
        driver: u16,
        residual: c_int,
        duration: u32,
        info: u32,
    }
    if std::fs::read_to_string(device.join("type")).ok()?.trim() != "5" {
        return None;
    }
    let node = std::path::Path::new("/dev").join(device.parent()?.file_name()?);
    // O_NONBLOCK permits metadata discovery with an empty tray.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(node)
        .ok()?;
    let query = |mut command: Vec<u8>, length: usize| -> Option<Vec<u8>> {
        let mut data = vec![0; length];
        let mut sense = [0u8; 32];
        let mut header = Header {
            interface_id: i32::from(b'S'),
            direction: -3,
            command_len: command.len() as u8,
            sense_len: 32,
            iovec_count: 0,
            length: length as u32,
            data: data.as_mut_ptr().cast(),
            command: command.as_mut_ptr(),
            sense: sense.as_mut_ptr(),
            timeout: 5000,
            flags: 0,
            pack_id: 0,
            user: std::ptr::null_mut(),
            status: 0,
            masked_status: 0,
            message: 0,
            written_sense: 0,
            host: 0,
            driver: 0,
            residual: 0,
            duration: 0,
            info: 0,
        };
        // Buffers and the SG_IO v3 header remain live until the synchronous call ends.
        if unsafe { libc::ioctl(file.as_raw_fd(), 0x2285, &mut header) } < 0
            || header.status != 0
            || header.host != 0
            || header.driver != 0
            || header.residual < 0
            || header.residual as usize > length
        {
            return None;
        }
        data.truncate(length - header.residual as usize);
        Some(data)
    };
    let inquiry = query(vec![0x12, 0, 0, 0, 36, 0], 36)?;
    if inquiry.len() < 36 || inquiry[0] & 31 != 5 {
        return None;
    }
    if let Some(identity) = query(vec![0x46, 2, 1, 8, 0, 0, 0, 2, 0, 0], 512)
        .and_then(|response| Identity::from_configuration(&inquiry, &response))
    {
        return Some(identity);
    }
    let page = query(vec![0x12, 1, 0x80, 0, 255, 0], 255)?;
    Identity::from_inquiry(&inquiry, &page)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_scsi_inquiry_matches_cached_fields_without_firmware_or_usb_serials() {
        let mut inquiry = [b' '; 36];
        inquiry[0] = 5;
        inquiry[8..16].copy_from_slice(b"HL-DT-ST");
        inquiry[16..28].copy_from_slice(b"DVDRW  GX40N");
        inquiry[32..36].copy_from_slice(b"RQ00");
        let page = [5, 0x80, 0, 6, b'K', b'1', b'2', b'3', b'4', b'5'];
        let cached = Identity {
            vendor: "HL-DT-ST".into(),
            product: "DVDRW GX40N".into(),
            serial: "K12345".into(),
        };
        let raw = Identity::from_inquiry(&inquiry, &page).unwrap();
        assert_eq!(raw.id(), cached.id());
        assert_eq!(raw.id().as_deref(), Some("9c9be5068b6f"));
        let mut configuration = vec![0, 0, 0, 14, 0, 0, 0, 0, 1, 8, 0, 6];
        configuration.extend_from_slice(b"K12345");
        assert_eq!(
            Identity::from_configuration(&inquiry, &configuration)
                .unwrap()
                .id(),
            raw.id()
        );
        inquiry[32..36].copy_from_slice(b"RQ99");
        assert_eq!(
            Identity::from_inquiry(&inquiry, &page).unwrap().id(),
            cached.id()
        );
        inquiry[0] = 0;
        assert!(Identity::from_inquiry(&inquiry, &page).is_none());
        assert!(Identity::from_inquiry(&inquiry[..31], &page).is_none());
    }
    #[test]
    fn hardware_id_ignores_os_paths_padding_and_firmware_but_preserves_serial() {
        let make = |vendor: &str, product: &str, serial: &str| Identity {
            vendor: vendor.into(),
            product: product.into(),
            serial: serial.into(),
        };
        let expected = make("HL-DT-ST", "DVDRW GX40N", "KABX128713").id().unwrap();
        for value in [
            make("HL-DT-ST ", "DVDRW   GX40N   ", " KABX128713\0"),
            make("hl-dt-st", "dvdrw gx40n", "KABX128713"),
        ] {
            assert_eq!(value.id().as_deref(), Some(expected.as_str()));
        }
        assert_ne!(
            make("HL-DT-ST", "DVDRW GX40N", "KABX128714").id().unwrap(),
            expected
        );
        for serial in [
            "",
            "00000000",
            "FFFFFFFF",
            "unknown",
            "1234567890",
            "Serial Number",
            "bad\nserial",
        ] {
            assert!(make("HL-DT-ST", "DVDRW GX40N", serial).id().is_none());
        }
    }
    #[test]
    fn vpd_serial_parser_checks_type_length_and_page() {
        let page = [5, 0x80, 0, 6, b'K', b'1', b'2', b'3', b'4', b'5'];
        assert_eq!(serial_page(&page).as_deref(), Some("K12345"));
        assert!(serial_page(&page[..9]).is_none());
        for (index, byte) in [(0, 0), (1, 0x83), (2, 1)] {
            let mut bad = page;
            bad[index] = byte;
            assert!(serial_page(&bad).is_none());
        }
    }

    #[test]
    fn mmc_serial_parser_accepts_only_complete_drive_serial_descriptors() {
        let response = [
            0, 0, 0, 14, 0, 0, 0, 0, 1, 8, 0, 6, b'K', b'1', b'2', b'3', b'4', b'5',
        ];
        assert_eq!(serial_feature(&response).as_deref(), Some("K12345"));
        assert!(serial_feature(&response[..17]).is_none());
        for (offset, value) in [(3, 7), (8, 0), (9, 9), (11, 7), (11, 0)] {
            let mut bad = response;
            bad[offset] = value;
            assert!(serial_feature(&bad).is_none());
        }
    }
}
