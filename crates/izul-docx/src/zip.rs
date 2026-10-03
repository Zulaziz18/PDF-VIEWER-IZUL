//! The smallest ZIP writer a .docx needs: deflated entries, one central
//! directory, no ZIP64, no encryption. Written here rather than taken from a
//! crate because those few hundred bytes of structure are all of it, and the
//! output is then byte-for-byte deterministic (fixed timestamps), which is what
//! makes the package testable.

use std::io::Write as _;

use flate2::write::DeflateEncoder;
use flate2::{Compression, Crc};

/// 1980-01-01 00:00, the earliest MS-DOS date: a fixed time keeps two
/// conversions of the same document identical.
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = (1 << 5) | 1;

struct Entry {
    name: String,
    crc: u32,
    compressed: u32,
    size: u32,
    method: u16,
    offset: u32,
}

#[derive(Default)]
pub(crate) struct ZipWriter {
    out: Vec<u8>,
    entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ZipError {
    /// Past 4 GiB, which needs ZIP64.
    TooLarge,
}

fn u16le(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn u32le(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

impl ZipWriter {
    /// Adds a file. Pictures that are already compressed (JPEG, PNG) are
    /// stored: deflating them again costs time and gains nothing.
    pub(crate) fn add(&mut self, name: &str, data: &[u8], compress: bool) -> Result<(), ZipError> {
        let mut crc = Crc::new();
        crc.update(data);
        let (method, body) = if compress {
            let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
            // Writing into a Vec cannot fail.
            let _ = enc.write_all(data);
            let packed = enc.finish().unwrap_or_default();
            (8u16, packed)
        } else {
            (0u16, data.to_vec())
        };
        let offset = u32::try_from(self.out.len()).map_err(|_| ZipError::TooLarge)?;
        let size = u32::try_from(data.len()).map_err(|_| ZipError::TooLarge)?;
        let compressed = u32::try_from(body.len()).map_err(|_| ZipError::TooLarge)?;
        let name_len = u16::try_from(name.len()).map_err(|_| ZipError::TooLarge)?;

        let o = &mut self.out;
        u32le(o, 0x0403_4b50);
        u16le(o, 20); // version needed: 2.0
        u16le(o, 0); // flags
        u16le(o, method);
        u16le(o, DOS_TIME);
        u16le(o, DOS_DATE);
        u32le(o, crc.sum());
        u32le(o, compressed);
        u32le(o, size);
        u16le(o, name_len);
        u16le(o, 0); // extra
        o.extend_from_slice(name.as_bytes());
        o.extend_from_slice(&body);

        self.entries.push(Entry {
            name: name.to_string(),
            crc: crc.sum(),
            compressed,
            size,
            method,
            offset,
        });
        Ok(())
    }

    pub(crate) fn finish(mut self) -> Result<Vec<u8>, ZipError> {
        let start = u32::try_from(self.out.len()).map_err(|_| ZipError::TooLarge)?;
        let count = u16::try_from(self.entries.len()).map_err(|_| ZipError::TooLarge)?;
        for e in &self.entries {
            let o = &mut self.out;
            u32le(o, 0x0201_4b50);
            u16le(o, 20); // made by
            u16le(o, 20); // needed
            u16le(o, 0);
            u16le(o, e.method);
            u16le(o, DOS_TIME);
            u16le(o, DOS_DATE);
            u32le(o, e.crc);
            u32le(o, e.compressed);
            u32le(o, e.size);
            u16le(o, e.name.len() as u16);
            u16le(o, 0); // extra
            u16le(o, 0); // comment
            u16le(o, 0); // disk
            u16le(o, 0); // internal attributes
            u32le(o, 0); // external attributes
            u32le(o, e.offset);
            o.extend_from_slice(e.name.as_bytes());
        }
        let end = u32::try_from(self.out.len()).map_err(|_| ZipError::TooLarge)?;
        let o = &mut self.out;
        u32le(o, 0x0605_4b50);
        u16le(o, 0);
        u16le(o, 0);
        u16le(o, count);
        u16le(o, count);
        u32le(o, end - start);
        u32le(o, start);
        u16le(o, 0);
        Ok(self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_well_formed_archive() {
        let mut z = ZipWriter::default();
        z.add("a.txt", b"halo halo halo halo", true).unwrap();
        z.add("b.bin", &[1, 2, 3], false).unwrap();
        let bytes = z.finish().unwrap();
        assert_eq!(bytes.get(..4), Some(&[0x50, 0x4b, 0x03, 0x04][..]));
        // End of central directory: two entries.
        let eocd = bytes.len() - 22;
        assert_eq!(
            bytes.get(eocd..eocd + 4),
            Some(&[0x50, 0x4b, 0x05, 0x06][..])
        );
        assert_eq!(bytes.get(eocd + 10..eocd + 12), Some(&[2, 0][..]));
    }

    #[test]
    fn is_deterministic() {
        let make = || {
            let mut z = ZipWriter::default();
            z.add("x.xml", b"<a/>", true).unwrap();
            z.finish().unwrap()
        };
        assert_eq!(make(), make());
    }
}
