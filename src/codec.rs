// SPDX-License-Identifier: GPL-3.0-or-later
// CIRC position mapping adapted from DiscImageStudio (revision 3a5cde635c182d6a7e01ff99c2cc1be2ac3ab668),
// Copyright (c) 2026 Disc Image Studio contributors, and CDImage,
// Copyright (C) 2008–2022 arduinocelentano. Rust adaptation: 2026 discimage contributors.
// Original CDImage revision: de5ca19f403b86934cfedb6793ca75f6716080c0/src/converter.cpp.
// Changes: Rust tables, absolute-session addressing and independent Mode 1 framing.
// Mode 1 framing, EDC/ECC and scrambling follow ECMA-130 clauses 14–17 / Annexes A–C.

use crate::project::Disc;

pub const COOKED: usize = 2048;
pub const RAW: usize = 2352;

pub struct Codec {
    pub scramble: [u8; RAW - 12],
    inverse: [i64; 24],
    edc: [u32; 256],
    forward: [u8; 256],
    backward: [u8; 256],
}

impl Default for Codec {
    fn default() -> Self {
        let mut result = Self {
            scramble: [0; RAW - 12],
            inverse: [0; 24],
            edc: [0; 256],
            forward: [0; 256],
            backward: [0; 256],
        };
        let mut state = 1u16;
        for byte in &mut result.scramble {
            for bit in 0..8 {
                *byte |= ((state & 1) as u8) << bit;
                state = (state >> 1) | (((state ^ (state >> 1)) & 1) << 14);
            }
        }
        for lane in 0..24i64 {
            let odd = lane % 2;
            let delay = if lane < 12 {
                odd - 24 * (lane * 4 + if odd == 1 { 2 } else { 3 })
                    + [0, 8, 16, 2, 10, 18][lane as usize / 2]
            } else {
                odd - 24 * ((lane + 4) * 4 + if odd == 1 { 0 } else { 1 })
                    + [4, 12, 20, 6, 14, 22][(lane as usize - 12) / 2]
            };
            let column = delay.rem_euclid(24);
            let displacement = 24 * 111 - (column - delay) + column - lane;
            result.inverse[(lane + displacement).rem_euclid(24) as usize] = -displacement;
        }
        for i in 0..256 {
            let mut crc = i as u32;
            for _ in 0..8 {
                crc = (crc >> 1) ^ if crc & 1 != 0 { 0xd8018001 } else { 0 };
            }
            result.edc[i] = crc;
            let doubled = ((i << 1) ^ if i & 128 != 0 { 0x11d } else { 0 }) as u8;
            result.forward[i] = doubled;
            result.backward[usize::from(doubled ^ i as u8)] = i as u8;
        }
        result
    }
}

impl Codec {
    /// Upstream logical-byte convention, including pair swap and inverse CIRC delay.
    /// This models payload positions only, not the parity bytes or complete optical waveform.
    pub fn physical(&self, lba: u32, offset: usize) -> i64 {
        let byte = (i64::from(lba) * RAW as i64 + 16 + offset as i64) ^ 1;
        byte + self.inverse[byte as usize % 24]
    }

    /// Raw Mode 1 image sector, UNSCRAMBLED. The drive performs scrambling/CIRC/EFM.
    pub fn sector(&self, lba: u32, payload: &[u8; COOKED]) -> [u8; RAW] {
        let mut raw = [0; RAW];
        raw[1..11].fill(0xff);
        let time = lba + 150;
        let bcd = |v: u32| ((v / 10) * 16 + v % 10) as u8;
        raw[12..16].copy_from_slice(&[bcd(time / 4500), bcd(time / 75 % 60), bcd(time % 75), 1]);
        raw[16..2064].copy_from_slice(payload);
        let crc = raw[..2064].iter().fold(0, |crc, b| {
            (crc >> 8) ^ self.edc[((crc ^ u32::from(*b)) & 255) as usize]
        });
        raw[2064..2068].copy_from_slice(&crc.to_le_bytes());
        let mut p = [0; 172];
        self.parity(&raw[12..2076], &mut p, 86, 24, 2, 86);
        raw[2076..2248].copy_from_slice(&p);
        let mut q = [0; 104];
        self.parity(&raw[12..2248], &mut q, 52, 43, 86, 88);
        raw[2248..].copy_from_slice(&q);
        raw
    }

    fn parity(
        &self,
        source: &[u8],
        output: &mut [u8],
        major: usize,
        minor: usize,
        mult: usize,
        inc: usize,
    ) {
        for row in 0..major {
            let mut index = (row / 2) * mult + row % 2;
            let (mut a, mut b) = (0, 0);
            for _ in 0..minor {
                let value = source[index];
                index = (index + inc) % (major * minor);
                a = self.forward[usize::from(a ^ value)];
                b ^= value;
            }
            a = self.backward[usize::from(self.forward[usize::from(a)] ^ b)];
            output[row] = a;
            output[row + major] = a ^ b;
        }
    }
}

/// Absolute disc coordinates; byte origin is first music INDEX 01, not session 2.
pub fn point(disc: &Disc, byte: i64) -> (f64, f64) {
    crate::spiral::Spiral::new(disc).point(byte)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn raw_mode1_matches_independent_edc_ecc_vector() {
        // Reference: bitwise EDC + libchdr's explicit P/Q row-offset tables.
        // See docs/validation.md for the independently generated vector provenance.
        let payload = std::array::from_fn(|i| i as u8);
        let raw = Codec::default().sector(12345, &payload);
        assert_eq!(
            crate::hex(&Sha256::digest(raw)),
            "891347f572c1920d48b495668d21e0123beefeb2ad454a83e3aa9e9c691da11f"
        );
        assert_eq!(&raw[12..16], &[0x02, 0x46, 0x45, 1]);
        assert_eq!(&raw[2068..2076], &[0; 8]);
    }

    #[test]
    fn scrambler_and_circ_lane_mapping_are_not_identity() {
        let codec = Codec::default();
        assert_eq!(
            &codec.scramble[..12],
            &[1, 128, 0, 96, 0, 40, 0, 30, 128, 8, 96, 6]
        );
        let mut residues = std::collections::HashSet::new();
        for lane in 0..24 {
            let position = codec.physical(12000, lane);
            assert!(position < 12000 * 2352 + 16 + lane as i64);
            residues.insert(position.rem_euclid(24));
        }
        assert_eq!(residues.len(), 24);
    }

    #[test]
    fn inverse_origin_is_111_frames_before_forward_circ() {
        // Independent forward path, ECMA-130 figure C.4. This checks the
        // coordinate convention used when comparing artwork with drive timing;
        // 111 is NOT a measured firmware/read-completion delay.
        let codec = Codec::default();
        let words = [0, 4, 8, 1, 5, 9, 2, 6, 10, 3, 7, 11];
        for lba in [0, 120520, 359000] {
            for offset in 0..COOKED {
                let swapped = (i64::from(lba) * RAW as i64 + 16 + offset as i64) ^ 1;
                let input = swapped.rem_euclid(24);
                let word = words.iter().position(|&w| w == input / 2).unwrap() as i64;
                let packed = word * 2 + input % 2;
                let slot = packed + if packed >= 12 { 4 } else { 0 };
                let delay =
                    slot * 4 + if slot < 12 { 2 } else { 0 } + if slot % 2 == 0 { 1 } else { 0 };
                let forward = swapped.div_euclid(24) + delay;
                assert_eq!(forward - codec.physical(lba, offset).div_euclid(24), 111);
            }
        }
    }
}
