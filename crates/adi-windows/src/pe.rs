use crate::Error;
use std::collections::BTreeMap;

#[derive(Debug)]
pub(crate) struct Image {
    pub memory: Vec<u8>,
    pub base: u64,
    pub entry: u64,
    pub imports: Vec<(u64, String)>,
    pub exports: BTreeMap<String, u64>,
}
impl Image {
    pub fn parse(bytes: Vec<u8>) -> Result<Self, Error> {
        let pe = goblin::pe::PE::parse(&bytes).map_err(|_| "invalid PE")?;
        let optional = pe
            .header
            .optional_header
            .as_ref()
            .ok_or("missing PE header")?;
        let size = optional.windows_fields.size_of_image as usize;
        let headers = optional.windows_fields.size_of_headers as usize;
        if !pe.is_64
            || !pe.is_lib
            || pe.header.coff_header.machine != 0x8664
            || pe.image_base != 0x7c800000
            || size == 0
            || size > 32 * 1024 * 1024
            || !size.is_multiple_of(4096)
            || pe.entry as usize >= size
            || headers > bytes.len()
            || headers > size
            || pe.sections.len() > 32
            || pe.imports.len() > 512
            || pe.tls_data.is_some()
        {
            return Err("unsupported PE layout".into());
        }
        let mut memory = vec![0; size];
        memory[..headers].copy_from_slice(&bytes[..headers]);
        let mut ranges = Vec::new();
        ranges.push(0..headers);
        for section in &pe.sections {
            let start = section.virtual_address as usize;
            let len = section.size_of_raw_data as usize;
            let end = start
                .checked_add(len.max(section.virtual_size as usize))
                .ok_or("PE section overflow")?;
            let offset = section.pointer_to_raw_data as usize;
            let source_end = offset.checked_add(len).ok_or("PE file overflow")?;
            if end > size
                || source_end > bytes.len()
                || ranges.iter().any(|r| start < r.end && r.start < end)
            {
                return Err("invalid PE section range".into());
            }
            memory[start..start + len].copy_from_slice(&bytes[offset..source_end]);
            ranges.push(start..end);
        }
        let mut imports = vec![];
        for import in &pe.imports {
            let address = import.offset as u64;
            if address.checked_add(8).is_none_or(|n| n > size as u64)
                || import.name.is_empty()
                || !["kernel32.dll", "advapi32.dll", "shlwapi.dll", "shell32.dll"]
                    .contains(&import.dll.to_ascii_lowercase().as_str())
            {
                return Err("unsupported PE import".into());
            }
            imports.push((address, import.name.to_string()));
        }
        let mut exports = BTreeMap::new();
        for export in &pe.exports {
            if export.reexport.is_some() || export.rva >= size {
                return Err("unsupported PE export".into());
            }
            if let Some(name) = export.name {
                exports.insert(name.to_string(), pe.image_base + export.rva as u64);
            }
        }
        Ok(Self {
            memory,
            base: pe.image_base,
            entry: pe.image_base + pe.entry as u64,
            imports,
            exports,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::Image;
    #[test]
    fn malformed_images_fail_closed() {
        for bytes in [vec![], vec![0; 512], b"MZ".to_vec()] {
            assert!(Image::parse(bytes).is_err());
        }
    }
}
