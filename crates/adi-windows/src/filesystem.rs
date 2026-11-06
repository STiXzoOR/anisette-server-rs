use crate::Error;
use std::collections::BTreeMap;
pub(crate) const ROOT: &str = "c:\\programdata\\apple computer\\itunes\\adi\\";
const MAX_FILE: usize = 1024 * 1024;
#[derive(Default)]
pub(crate) struct FileSystem {
    pub files: BTreeMap<String, Vec<u8>>,
    handles: BTreeMap<u64, Handle>,
}
struct Handle {
    path: String,
    offset: usize,
    access: u32,
    share: u32,
}
pub(crate) fn valid_path(path: &str) -> bool {
    path.strip_prefix(ROOT).is_some_and(|name| {
        !name.is_empty()
            && name.len() <= 100
            && name.ends_with(".pb")
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
            && !name.contains("..")
    })
}
impl FileSystem {
    pub fn open(
        &mut self,
        path: String,
        access: u32,
        share: u32,
        creation: u32,
    ) -> Result<u64, u32> {
        if !valid_path(&path)
            || access & !0xc0000080 != 0
            || share & !7 != 0
            || !(1..=5).contains(&creation)
        {
            return Err(87);
        }
        if self.handles.len() >= 64 {
            return Err(4);
        }
        let exists = self.files.contains_key(&path);
        if creation == 1 && exists {
            return Err(80);
        }
        if (creation == 3 || creation == 5) && !exists {
            return Err(2);
        }
        if creation == 5 && access & 0x40000000 == 0 {
            return Err(5);
        }
        for handle in self.handles.values().filter(|h| h.path == path) {
            if (access >> 31 & 1 != 0 && handle.share & 1 == 0)
                || (access >> 30 & 1 != 0 && handle.share & 2 == 0)
                || (handle.access >> 31 & 1 != 0 && share & 1 == 0)
                || (handle.access >> 30 & 1 != 0 && share & 2 == 0)
            {
                return Err(32);
            }
        }
        if !exists && self.files.len() >= 32 {
            return Err(112);
        }
        if creation == 2 || creation == 5 || !exists {
            self.files.insert(path.clone(), vec![]);
        }
        let id = (0x9000..0x9040)
            .find(|id| !self.handles.contains_key(id))
            .ok_or(4u32)?;
        self.handles.insert(
            id,
            Handle {
                path,
                offset: 0,
                access,
                share,
            },
        );
        Ok(id)
    }
    pub fn close(&mut self, id: u64) -> bool {
        self.handles.remove(&id).is_some()
    }
    pub fn contains(&self, id: u64) -> bool {
        self.handles.contains_key(&id)
    }
    pub fn file_id(&self, id: u64) -> Result<u64, Error> {
        use sha2::{Digest, Sha256};
        let h = self.handles.get(&id).ok_or("invalid file handle")?;
        let digest = Sha256::digest(h.path.as_bytes());
        let bytes: [u8; 8] = digest[..8].try_into().expect("SHA-256 has 32 bytes");
        Ok(u64::from_le_bytes(bytes))
    }
    pub fn size(&self, id: u64) -> Result<usize, Error> {
        let h = self.handles.get(&id).ok_or("invalid file handle")?;
        Ok(self.files[&h.path].len())
    }
    pub fn read(&mut self, id: u64, count: usize) -> Result<Vec<u8>, Error> {
        if count > MAX_FILE {
            return Err("file read exceeds limit".into());
        }
        let h = self.handles.get_mut(&id).ok_or("invalid file handle")?;
        if h.access & 0x80000000 == 0 {
            return Err("file not readable".into());
        }
        let bytes = &self.files[&h.path];
        let start = h.offset.min(bytes.len());
        let end = (start + count).min(bytes.len());
        h.offset += end - start;
        Ok(bytes[start..end].to_vec())
    }
    pub fn write(&mut self, id: u64, bytes: &[u8]) -> Result<(), Error> {
        let total: usize = self.files.values().map(Vec::len).sum();
        let h = self.handles.get_mut(&id).ok_or("invalid file handle")?;
        if h.access & 0x40000000 == 0 {
            return Err("file not writable".into());
        }
        let end = h
            .offset
            .checked_add(bytes.len())
            .filter(|n| *n <= MAX_FILE)
            .ok_or("file too large")?;
        let file = self.files.get_mut(&h.path).ok_or("invalid file handle")?;
        if total + end.saturating_sub(file.len()) > 4 * MAX_FILE {
            return Err("file aggregate limit".into());
        }
        file.resize(file.len().max(end), 0);
        file[h.offset..end].copy_from_slice(bytes);
        h.offset = end;
        Ok(())
    }
    pub fn seek(&mut self, id: u64, distance: i64, origin: u32) -> Result<usize, Error> {
        let h = self.handles.get_mut(&id).ok_or("invalid file handle")?;
        let base = match origin {
            0 => 0,
            1 => h.offset,
            2 => self.files[&h.path].len(),
            _ => return Err("invalid seek origin".into()),
        };
        h.offset = (base as i64)
            .checked_add(distance)
            .filter(|n| *n >= 0 && *n <= MAX_FILE as i64)
            .ok_or("invalid seek offset")? as usize;
        Ok(h.offset)
    }
    pub fn truncate(&mut self, id: u64) -> Result<(), Error> {
        let total: usize = self.files.values().map(Vec::len).sum();
        let h = self.handles.get(&id).ok_or("invalid file handle")?;
        if h.access & 0x40000000 == 0 {
            return Err("file not writable".into());
        }
        let file = self.files.get_mut(&h.path).ok_or("invalid file handle")?;
        if total + h.offset.saturating_sub(file.len()) > 4 * MAX_FILE {
            return Err("file aggregate limit".into());
        }
        file.resize(h.offset, 0);
        Ok(())
    }
    pub fn delete(&mut self, path: &str) -> Result<(), u32> {
        if !valid_path(path) {
            return Err(87);
        }
        if self.handles.values().any(|h| h.path == path) {
            return Err(32);
        }
        self.files.remove(path).map(|_| ()).ok_or(2)
    }
}
#[cfg(test)]
mod tests {
    use super::{FileSystem, MAX_FILE, ROOT};
    #[test]
    fn private_paths_handles_offsets_sharing_and_limits() {
        let mut fs = FileSystem::default();
        let path = format!("{ROOT}adi-test.pb");
        assert!(
            fs.open(format!("{ROOT}..\\test.pb"), 0xc0000000, 3, 2)
                .is_err()
        );
        assert_eq!(fs.open(path.clone(), 0x80000000, 3, 3), Err(2));
        let h = fs.open(path.clone(), 0xc0000000, 0, 2).unwrap();
        assert_eq!(fs.open(path.clone(), 0x80000000, 3, 3), Err(32));
        fs.write(h, b"test").unwrap();
        fs.seek(h, -2, 1).unwrap();
        fs.write(h, b"!").unwrap();
        fs.seek(h, 0, 0).unwrap();
        assert_eq!(fs.read(h, 100).unwrap(), b"te!t");
        assert!(fs.seek(h, -5, 0).is_err());
        assert!(fs.write(h, &vec![0; MAX_FILE + 1]).is_err());
        assert_eq!(fs.delete(&path), Err(32));
        assert!(fs.close(h));
        assert!(!fs.close(h));
        assert!(fs.read(h, 1).is_err());
        fs.delete(&path).unwrap();
    }
}
