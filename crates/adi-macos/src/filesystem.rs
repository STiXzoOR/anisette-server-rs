// SPDX-License-Identifier: GPL-2.0-only
// macOS stat layout and virtual FS behavior adapted from os-findmy-e stubs.py.
use std::collections::{BTreeMap, BTreeSet};
const MAX_FILES: usize = 64;
const MAX_BYTES: usize = 4 * 1024 * 1024;
struct Descriptor {
    path: String,
    offset: usize,
    flags: u64,
}
pub(crate) struct FileSystem {
    pub files: BTreeMap<String, Vec<u8>>,
    pub dirs: BTreeSet<String>,
    descriptors: BTreeMap<u64, Descriptor>,
    next_fd: u64,
}
impl Default for FileSystem {
    fn default() -> Self {
        Self {
            files: BTreeMap::from([("/var/db/.AppleSetupUser".into(), vec![])]),
            dirs: [
                "/",
                "/Users",
                "/Users/Shared",
                "/var",
                "/var/db",
                "/var/db/fpsd",
                "/var/db/fpsd/adi",
            ]
            .map(String::from)
            .into(),
            descriptors: BTreeMap::new(),
            next_fd: 3,
        }
    }
}
impl FileSystem {
    pub fn valid_path(path: &str) -> bool {
        path.starts_with('/')
            && path.len() <= 4096
            && !path.contains('\0')
            && (path == "/"
                || !path[1..]
                    .split('/')
                    .any(|s| s.is_empty() || s == "." || s == ".."))
    }
    pub fn exists(&self, path: &str) -> bool {
        self.files.contains_key(path) || self.dirs.contains(path)
    }
    fn resize(&mut self, path: &str, size: usize) -> Result<(), i32> {
        let total: usize = self.files.values().map(Vec::len).sum();
        let file = self.files.get_mut(path).ok_or(2)?;
        if size > 1024 * 1024 || total - file.len() + size > MAX_BYTES {
            return Err(28);
        }
        file.resize(size, 0);
        Ok(())
    }
    pub fn open(&mut self, path: String, flags: u64) -> Result<u64, i32> {
        if !Self::valid_path(&path) {
            return Err(22);
        }
        if self.descriptors.len() >= 64 {
            return Err(24);
        }
        if self.dirs.contains(&path) {
            return Err(21);
        }
        if !self.files.contains_key(&path) {
            if flags & 0x200 == 0 {
                return Err(2);
            }
            if self.files.len() >= MAX_FILES {
                return Err(28);
            }
            let (parent, _) = path.rsplit_once('/').ok_or(22)?;
            if !self
                .dirs
                .contains(if parent.is_empty() { "/" } else { parent })
            {
                return Err(2);
            }
            self.files.insert(path.clone(), vec![]);
        }
        if flags & 0x400 != 0 {
            if flags & 3 == 0 {
                return Err(9);
            }
            self.files.get_mut(&path).ok_or(2)?.clear();
        }
        let fd = self.next_fd;
        self.next_fd = self.next_fd.checked_add(1).ok_or(24)?;
        self.descriptors.insert(
            fd,
            Descriptor {
                path,
                offset: 0,
                flags,
            },
        );
        Ok(fd)
    }
    pub fn close(&mut self, fd: u64) -> Result<(), i32> {
        self.descriptors.remove(&fd).map(|_| ()).ok_or(9)
    }
    pub fn read(&mut self, fd: u64, n: usize) -> Result<Vec<u8>, i32> {
        let d = self.descriptors.get_mut(&fd).ok_or(9)?;
        if d.flags & 3 == 1 {
            return Err(9);
        }
        let data = self.files.get(&d.path).ok_or(2)?;
        let start = d.offset.min(data.len());
        let end = start.saturating_add(n).min(data.len());
        d.offset = end;
        Ok(data[start..end].to_vec())
    }
    pub fn write(&mut self, fd: u64, bytes: &[u8]) -> Result<usize, i32> {
        let d = self.descriptors.get(&fd).ok_or(9)?;
        if d.flags & 3 == 0 {
            return Err(9);
        }
        let path = d.path.clone();
        let offset = d.offset;
        let end = offset.checked_add(bytes.len()).ok_or(28)?;
        self.resize(&path, end.max(self.files[&path].len()))?;
        self.files.get_mut(&path).ok_or(2)?[offset..end].copy_from_slice(bytes);
        self.descriptors.get_mut(&fd).ok_or(9)?.offset = end;
        Ok(bytes.len())
    }
    pub fn truncate(&mut self, fd: u64, size: usize) -> Result<(), i32> {
        let d = self.descriptors.get(&fd).ok_or(9)?;
        if d.flags & 3 == 0 {
            return Err(9);
        }
        self.resize(&d.path.clone(), size)
    }
    pub fn mkdir(&mut self, path: String) -> Result<(), i32> {
        if !Self::valid_path(&path) {
            return Err(22);
        }
        if self.exists(&path) {
            return Err(17);
        }
        if self.dirs.len() >= MAX_FILES {
            return Err(28);
        }
        let (parent, _) = path.rsplit_once('/').ok_or(22)?;
        if !self
            .dirs
            .contains(if parent.is_empty() { "/" } else { parent })
        {
            return Err(2);
        }
        self.dirs.insert(path);
        Ok(())
    }
    pub fn unlink(&mut self, path: &str) -> Result<(), i32> {
        if self.descriptors.values().any(|d| d.path == path) {
            return Err(16);
        }
        self.files.remove(path).map(|_| ()).ok_or(2)
    }
    pub fn stat_fd(&self, fd: u64) -> Result<[u8; 144], i32> {
        self.stat(&self.descriptors.get(&fd).ok_or(9)?.path)
    }
    pub fn stat(&self, path: &str) -> Result<[u8; 144], i32> {
        if !Self::valid_path(path) {
            return Err(22);
        }
        let dir = self.dirs.contains(path);
        let size = if dir {
            0
        } else {
            self.files.get(path).ok_or(2)?.len()
        };
        let mut crc = !0u32;
        for b in path.bytes() {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb88320 } else { 0 };
            }
        }
        let mut raw = [0u8; 144];
        raw[..4].copy_from_slice(&0x1000003u32.to_le_bytes());
        raw[4..6]
            .copy_from_slice(&((if dir { 0o040000u16 } else { 0o100000 }) | 0o755).to_le_bytes());
        raw[6..8].copy_from_slice(&(if dir { 2u16 } else { 1 }).to_le_bytes());
        raw[8..16].copy_from_slice(&(0x10000 + u64::from(!crc & 0xfffff)).to_le_bytes());
        raw[16..20].copy_from_slice(&501u32.to_le_bytes());
        raw[20..24].copy_from_slice(&20u32.to_le_bytes());
        raw[96..104].copy_from_slice(&(size as u64).to_le_bytes());
        raw[104..112].copy_from_slice(&(size.div_ceil(512) as u64).to_le_bytes());
        raw[112..116].copy_from_slice(&4096u32.to_le_bytes());
        Ok(raw)
    }
}
#[cfg(test)]
mod tests {
    use super::{FileSystem, MAX_BYTES};
    #[test]
    fn file_lifecycle_permissions_and_limits() {
        let mut fs = FileSystem::default();
        let path = "/var/db/fpsd/adi/test".to_owned();
        assert_eq!(fs.open(path.clone(), 0), Err(2));
        let fd = fs.open(path.clone(), 0x202).unwrap();
        fs.write(fd, b"abc").unwrap();
        fs.truncate(fd, 6).unwrap();
        fs.close(fd).unwrap();
        let fd = fs.open(path.clone(), 0).unwrap();
        assert_eq!(fs.read(fd, 100).unwrap(), b"abc\0\0\0");
        assert_eq!(fs.write(fd, b"x"), Err(9));
        assert_eq!(fs.truncate(fd, 0), Err(9));
        fs.close(fd).unwrap();
        assert_eq!(fs.close(fd), Err(9));
        let fd = fs.open(path.clone(), 2).unwrap();
        assert_eq!(fs.truncate(fd, MAX_BYTES), Err(28));
        fs.close(fd).unwrap();
        fs.unlink(&path).unwrap();
        assert!(!fs.exists(&path));
    }
    #[test]
    fn paths_never_escape_or_alias() {
        for p in ["../a", "/var/../a", "/var//a", "/var/./a", "/a\0b"] {
            assert!(!FileSystem::valid_path(p));
        }
        let fs = FileSystem::default();
        assert!(fs.stat("/var/db/.AppleSetupUser").is_ok());
        assert_ne!(
            fs.stat("/var").unwrap()[8..16],
            fs.stat("/var/db").unwrap()[8..16]
        );
    }
}
