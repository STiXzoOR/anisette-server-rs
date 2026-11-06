use crate::Error;
use goblin::elf::{
    Elf,
    header::{EM_AARCH64, ET_DYN},
    program_header::PT_LOAD,
};
use std::{collections::BTreeMap, ops::Range};

#[derive(Debug)]
pub(crate) struct Image {
    pub bytes: Vec<u8>,
    pub size: u64,
    pub segments: Vec<(u64, Range<usize>)>,
    pub symbols: Vec<(String, Option<u64>)>,
    pub exports: BTreeMap<String, u64>,
    pub relocations: Vec<goblin::elf::reloc::Reloc>,
}
impl Image {
    pub fn parse(bytes: Vec<u8>) -> Result<Self, Error> {
        let elf = Elf::parse(&bytes).map_err(|_| "invalid ELF")?;
        if !elf.is_64
            || !elf.little_endian
            || elf.header.e_machine != EM_AARCH64
            || elf.header.e_type != ET_DYN
        {
            return Err("expected ARM64 little-endian ELF shared library".into());
        }
        let mut size = 0;
        let mut segments = vec![];
        let mut mapped: Vec<Range<u64>> = vec![];
        for p in elf.program_headers.iter().filter(|p| p.p_type == PT_LOAD) {
            let end = p
                .p_vaddr
                .checked_add(p.p_memsz)
                .filter(|n| *n <= 64 * 1024 * 1024)
                .ok_or("ELF mapping too large")?;
            let file_end = p
                .p_offset
                .checked_add(p.p_filesz)
                .filter(|n| *n <= bytes.len() as u64)
                .ok_or("ELF segment outside file")?;
            if p.p_filesz > p.p_memsz {
                return Err("ELF file segment exceeds mapping".into());
            }
            if mapped.iter().any(|r| p.p_vaddr < r.end && r.start < end) {
                return Err("overlapping ELF segments".into());
            }
            size = size.max(end);
            mapped.push(p.p_vaddr..end);
            segments.push((p.p_vaddr, p.p_offset as usize..file_end as usize));
        }
        if size == 0 {
            return Err("ELF has no loadable image".into());
        }
        let mut symbols = vec![];
        let mut exports = BTreeMap::new();
        if elf.dynsyms.len() > 16384 {
            return Err("too many ELF symbols".into());
        }
        for s in &elf.dynsyms {
            let name = elf
                .dynstrtab
                .get_at(s.st_name)
                .ok_or("invalid ELF symbol name")?
                .to_owned();
            let address = if s.st_shndx == 0 {
                None
            } else {
                if s.st_value >= size {
                    return Err("ELF export outside mapping".into());
                }
                exports.insert(name.clone(), s.st_value);
                Some(s.st_value)
            };
            symbols.push((name, address));
        }
        if !elf.dynrels.is_empty() {
            return Err("unsupported ELF REL encoding".into());
        }
        let relocations: Vec<_> = elf.dynrelas.iter().chain(elf.pltrelocs.iter()).collect();
        if relocations.len() > 100000 {
            return Err("too many ELF relocations".into());
        }
        for r in &relocations {
            let end = r.r_offset.checked_add(8).ok_or("ELF relocation overflow")?;
            if !mapped.iter().any(|p| p.start <= r.r_offset && end <= p.end)
                || r.r_sym >= symbols.len()
                || !matches!(r.r_type, 257 | 1025 | 1026 | 1027)
            {
                return Err("unsupported or out-of-range ELF relocation".into());
            }
        }
        Ok(Self {
            size: size.div_ceil(4096) * 4096,
            segments,
            symbols,
            exports,
            relocations,
            bytes,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::Image;
    #[test]
    fn malformed_elf_fails_closed() {
        for bytes in [vec![], b"\x7fELF".to_vec(), vec![0; 128]] {
            assert!(Image::parse(bytes).is_err());
        }
    }
}
