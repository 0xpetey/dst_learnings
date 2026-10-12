//! CRC-32C, the checksum on every log record.
//!
//! A checksum is the only way a reader can tell "the bytes I asked for" from
//! "the bytes the disk gave me". Without one, a flipped bit in a value is
//! served to a client as if it were the value. A flipped bit in a length
//! prefix makes recovery decode garbage as an entry. Module 6's fault model
//! produces both, on purpose.
//!
//! CRC-32C (Castagnoli) is the polynomial that iSCSI, ext4, Btrfs and
//! FoundationDB use, and that x86 and ARM compute in one instruction. This
//! is the table-driven software version, written out so that no dependency
//! can ever change what a stored checksum means. It is not cryptographic
//! and does not need to be: the adversary here is a disk, not a person.

/// The reversed Castagnoli polynomial.
const POLY: u32 = 0x82F6_3B78;

/// One table lookup per byte.
const TABLE: [u32; 256] = build_table();

const fn build_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ POLY } else { crc >> 1 };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

/// CRC-32C of `bytes`.
pub fn crc32c(bytes: &[u8]) -> u32 {
    crc32c_update(0, bytes)
}

/// Continue a checksum. `crc32c_update(crc32c(a), b) == crc32c(a ++ b)`,
/// which lets a record be checksummed without first copying it into one
/// buffer.
pub fn crc32c_update(crc: u32, bytes: &[u8]) -> u32 {
    let mut crc = !crc;
    for &b in bytes {
        crc = TABLE[usize::from((crc as u8) ^ b)] ^ (crc >> 8);
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_value() {
        // The "check" value every CRC-32C implementation agrees on.
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
        assert_eq!(crc32c(b""), 0);
        // From RFC 3720 (iSCSI), appendix B.4: 32 bytes of zeros.
        assert_eq!(crc32c(&[0u8; 32]), 0x8A91_36AA);
    }

    #[test]
    fn update_is_concatenation() {
        let a = b"hello, ";
        let b = b"world";
        assert_eq!(crc32c_update(crc32c(a), b), crc32c(b"hello, world"));
    }

    #[test]
    fn a_single_bit_flip_changes_the_sum() {
        let data = [7u8; 64];
        let sum = crc32c(&data);
        for i in 0..data.len() {
            for bit in 0..8 {
                let mut flipped = data;
                flipped[i] ^= 1 << bit;
                assert_ne!(crc32c(&flipped), sum, "byte {i} bit {bit}");
            }
        }
    }
}
