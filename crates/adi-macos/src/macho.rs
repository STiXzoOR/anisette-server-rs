// SPDX-License-Identifier: GPL-2.0-only
// Derived from sacca97/os-findmy-e native/adi-core/src/macho.rs at d4d2a83.
// Bounds checking and executable/PAGEZERO handling adapted for the pinned adid image.
use crate::Error;

const MAX_SEGMENTS: usize = 64;
const MAX_BINDINGS: usize = 16384;
const MAX_MAPPED_BYTES: u64 = 64 * 1024 * 1024;
fn invalid(message: &'static str) -> Error {
    Error::InvalidImage(message)
}
fn bytes(d: &[u8], o: usize, n: usize) -> Result<&[u8], Error> {
    d.get(o..o.checked_add(n).ok_or(invalid("offset overflow"))?)
        .ok_or(invalid("truncated image"))
}
fn u32_at(d: &[u8], o: usize) -> Result<u32, Error> {
    let b = bytes(d, o, 4)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}
fn u64_at(d: &[u8], o: usize) -> Result<u64, Error> {
    let b = bytes(d, o, 8)?;
    Ok(u64::from_le_bytes([
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
    ]))
}
fn leb(d: &[u8], o: &mut usize, signed: bool) -> Result<u64, Error> {
    let mut value = 0u64;
    for i in 0..10 {
        let b = *d.get(*o).ok_or(invalid("truncated LEB"))?;
        *o += 1;
        if i == 9 && if signed { b != 0 && b != 0x7f } else { b > 1 } {
            return Err(invalid("LEB overflow"));
        }
        value |= u64::from(b & 0x7f) << (i * 7);
        if b & 0x80 == 0 {
            if signed && i < 9 && b & 0x40 != 0 {
                value |= u64::MAX << ((i + 1) * 7);
            }
            return Ok(value);
        }
    }
    Err(invalid("LEB overflow"))
}

#[derive(Debug)]
pub struct Segment {
    pub name: String,
    pub address: u64,
    pub size: u64,
    pub file_offset: usize,
    pub file_size: usize,
    pub protection: u32,
}
#[derive(Debug)]
pub struct Bind {
    pub address: u64,
    pub symbol: String,
    pub addend: i64,
}
#[derive(Debug)]
pub struct Image {
    pub segments: Vec<Segment>,
    pub bindings: Vec<Bind>,
}

impl Image {
    pub(crate) fn parse(data: &[u8]) -> Result<Self, Error> {
        if u32_at(data, 0)? != 0xfeedfacf
            || u32_at(data, 4)? != 0x01000007
            || u32_at(data, 12)? != 2
        {
            return Err(invalid("expected thin x86_64 Mach-O executable"));
        }
        let count = u32_at(data, 16)? as usize;
        let commands = bytes(data, 32, u32_at(data, 20)? as usize)?;
        if count > 1024 {
            return Err(invalid("too many load commands"));
        }
        let mut offset = 0;
        let mut segments = Vec::new();
        let mut streams = Vec::new();
        let mut mapped = 0;
        for _ in 0..count {
            let kind = u32_at(commands, offset)?;
            let size = u32_at(commands, offset + 4)? as usize;
            if size < 8 || !size.is_multiple_of(8) {
                return Err(invalid("invalid load command size"));
            }
            let c = bytes(commands, offset, size)?;
            offset += size;
            if kind == 0x19 {
                let name = bytes(c, 8, 16)?;
                let name = std::str::from_utf8(name.split(|b| *b == 0).next().unwrap_or(name))
                    .map_err(|_| invalid("invalid segment name"))?
                    .to_owned();
                let address = u64_at(c, 24)?;
                let size = u64_at(c, 32)?;
                let file_offset =
                    usize::try_from(u64_at(c, 40)?).map_err(|_| invalid("file offset overflow"))?;
                let file_size =
                    usize::try_from(u64_at(c, 48)?).map_err(|_| invalid("file size overflow"))?;
                let protection = u32_at(c, 60)?;
                if segments.len() == MAX_SEGMENTS {
                    return Err(invalid("too many segments"));
                }
                if name != "__PAGEZERO" {
                    if size < file_size as u64
                        || size > MAX_MAPPED_BYTES
                        || !address.is_multiple_of(4096)
                    {
                        return Err(invalid("invalid segment mapping"));
                    }
                    let end = address
                        .checked_add(size)
                        .ok_or(invalid("segment overflow"))?;
                    for s in &segments {
                        let s: &Segment = s;
                        if s.name != "__PAGEZERO" && address < s.address + s.size && s.address < end
                        {
                            return Err(invalid("overlapping segments"));
                        }
                    }
                    mapped += size;
                    if mapped > MAX_MAPPED_BYTES {
                        return Err(invalid("image mapping exceeds limit"));
                    }
                    bytes(data, file_offset, file_size)?;
                } else if protection != 0 || file_size != 0 {
                    return Err(invalid("invalid PAGEZERO"));
                }
                segments.push(Segment {
                    name,
                    address,
                    size,
                    file_offset,
                    file_size,
                    protection,
                });
            } else if kind == 0x22 || kind == 0x80000022 {
                for (a, b, lazy) in [(16, 20, false), (24, 28, false), (32, 36, true)] {
                    streams.push((u32_at(c, a)? as usize, u32_at(c, b)? as usize, lazy));
                }
            } else if kind == 0x80000034 {
                return Err(invalid("chained fixups unsupported"));
            }
        }
        if offset != commands.len() {
            return Err(invalid("load command size mismatch"));
        }
        let mut image = Self {
            segments,
            bindings: Vec::new(),
        };
        for (offset, len, lazy) in streams {
            image.bind(bytes(data, offset, len)?, lazy)?;
        }
        Ok(image)
    }

    fn bind(&mut self, d: &[u8], lazy: bool) -> Result<(), Error> {
        let (mut o, mut segment, mut offset, mut addend) = (0, 0usize, 0u64, 0i64);
        let mut symbol = String::new();
        while o < d.len() {
            let b = d[o];
            o += 1;
            let imm = b & 15;
            match b & 0xf0 {
                0x00 => {
                    if !lazy {
                        break;
                    }
                }
                0x10 | 0x30 => {}
                0x20 => {
                    leb(d, &mut o, false)?;
                }
                0x40 => {
                    let len = d[o..]
                        .iter()
                        .position(|b| *b == 0)
                        .ok_or(invalid("unterminated symbol"))?;
                    symbol = std::str::from_utf8(&d[o..o + len])
                        .map_err(|_| invalid("invalid symbol"))?
                        .to_owned();
                    o += len + 1;
                }
                0x50 => {
                    if imm != 1 {
                        return Err(invalid("unsupported bind type"));
                    }
                }
                0x60 => {
                    addend = leb(d, &mut o, true)? as i64;
                }
                0x70 => {
                    segment = imm as usize;
                    offset = leb(d, &mut o, false)?;
                }
                0x80 => {
                    // dyld encodes backwards deltas in unsigned LEB; validate the resulting bind address below.
                    offset = offset.wrapping_add(leb(d, &mut o, false)?);
                }
                0x90 | 0xa0 | 0xb0 | 0xc0 => {
                    let (count, skip) = match b & 0xf0 {
                        0xa0 => (1, leb(d, &mut o, false)?),
                        0xb0 => (1, u64::from(imm) * 8),
                        0xc0 => (leb(d, &mut o, false)?, leb(d, &mut o, false)?),
                        _ => (1, 0),
                    };
                    if count > MAX_BINDINGS as u64
                        || self.bindings.len() + count as usize > MAX_BINDINGS
                    {
                        return Err(invalid("too many bindings"));
                    }
                    for _ in 0..count {
                        let s = self
                            .segments
                            .get(segment)
                            .ok_or(invalid("bad bind segment"))?;
                        if s.name == "__PAGEZERO"
                            || symbol.is_empty()
                            || offset.checked_add(8).is_none_or(|end| end > s.size)
                        {
                            return Err(invalid("binding outside segment"));
                        }
                        self.bindings.push(Bind {
                            address: s.address + offset,
                            symbol: symbol.clone(),
                            addend,
                        });
                        offset = offset.wrapping_add(8).wrapping_add(skip);
                    }
                }
                _ => return Err(invalid("unsupported bind opcode")),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Image, Segment, leb};
    #[test]
    fn rejects_malformed_commands_without_panics() {
        for len in 0..128 {
            assert!(Image::parse(&vec![0; len]).is_err());
        }
        let mut d = vec![0; 40];
        for (off, v) in [
            (0, 0xfeedfacfu32),
            (4, 0x01000007),
            (12, 2),
            (16, 1),
            (20, 8),
        ] {
            d[off..off + 4].copy_from_slice(&v.to_le_bytes());
        }
        assert!(Image::parse(&d).is_err());
        d[36..40].copy_from_slice(&8u32.to_le_bytes());
        assert!(Image::parse(&d).is_ok());
    }
    #[test]
    fn leb_boundaries_and_binding_limits() {
        assert_eq!(leb(&[0x7f], &mut 0, true).unwrap() as i64, -1);
        assert!(leb(&[0x80; 10], &mut 0, false).is_err());
        let mut image = Image {
            segments: vec![Segment {
                name: "__DATA".into(),
                address: 0x1000,
                size: 16,
                file_offset: 0,
                file_size: 0,
                protection: 3,
            }],
            bindings: vec![],
        };
        image.bind(b"\x40foo\0\x70\0\x90\0", false).unwrap();
        assert_eq!(image.bindings[0].address, 0x1000);
        // Offset 8 plus encoded -8 returns to the start of the segment.
        image
            .bind(
                b"\x40back\0\x70\x08\x80\xf8\xff\xff\xff\xff\xff\xff\xff\xff\x01\x90",
                false,
            )
            .unwrap();
        assert_eq!(image.bindings.last().unwrap().address, 0x1000);

        assert!(image.bind(b"\x40foo\0\x70\x10\x90", false).is_err());
        assert!(
            image
                .bind(b"\x40foo\0\x70\0\xc0\xff\xff\x7f\0", false)
                .is_err()
        );
        assert!(image.bind(b"\x40missingnul", false).is_err());
    }
}
