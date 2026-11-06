use crate::{BINARY_SHA256, Binary, Error, Runtime, filesystem::FileSystem};
use anisette::protocol::StateSnapshot;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::collections::BTreeMap;

const PROVIDER: &str = "macos-adid";

fn filesystem(files: &BTreeMap<String, String>) -> Result<FileSystem, Error> {
    if files.len() > 64 {
        return Err(Error::Execution("too many state files"));
    }
    let mut fs = FileSystem::default();
    for (path, encoded) in files {
        if !FileSystem::valid_path(path) || !path.starts_with("/var/db/fpsd/adi/") {
            return Err(Error::Execution("invalid state path"));
        }
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|_| Error::Execution("invalid state encoding"))?;
        let fd = fs
            .open(path.clone(), 0x202)
            .map_err(|_| Error::Execution("invalid state file"))?;
        fs.write(fd, &bytes)
            .map_err(|_| Error::Execution("state file exceeds limit"))?;
        fs.close(fd)
            .map_err(|_| Error::Execution("state close failed"))?;
    }
    Ok(fs)
}

impl Runtime {
    /// Exports the device state for later restoration.
    ///
    /// # Errors
    /// Returns an error if the runtime is poisoned, serialization fails, or the snapshot exceeds the state limit.
    pub fn export_state(&self, identifier: [u8; 16]) -> Result<Vec<u8>, Error> {
        self.ensure_usable()?;
        let data = self.cpu.get_data();
        let files = data
            .fs
            .files
            .iter()
            .filter(|(p, _)| p.starts_with("/var/db/fpsd/adi/"))
            .map(|(p, b)| (p.clone(), STANDARD.encode(b)))
            .collect();
        StateSnapshot::encode(PROVIDER, BINARY_SHA256, identifier, data.host_uuid, files)
            .map_err(Error::Execution)
    }
    /// Restores a device from a backend-specific state snapshot.
    ///
    /// # Errors
    /// Returns an error for invalid state size, identity, version, binary hash, or files, or if backend initialization fails.
    pub fn restore(binary: &Binary, identifier: [u8; 16], bytes: &[u8]) -> Result<Self, Error> {
        let state = StateSnapshot::decode(bytes, PROVIDER, BINARY_SHA256, identifier)
            .map_err(Error::Execution)?;
        let fs = filesystem(&state.files)?;
        let mut runtime = Self::new(binary)?;
        runtime.cpu.get_data_mut().host_uuid = state.host_uuid;
        runtime.cpu.get_data_mut().fs = fs;
        runtime.boot()?;
        Ok(runtime)
    }
}
#[cfg(test)]
mod tests {
    use super::{BINARY_SHA256, BTreeMap, PROVIDER, STANDARD, StateSnapshot, filesystem};
    use base64::Engine as _;
    #[test]
    fn rejects_wrong_identity_backend_version_and_paths() {
        let bytes =
            StateSnapshot::encode(PROVIDER, BINARY_SHA256, [1; 16], [2; 16], BTreeMap::new())
                .unwrap();
        assert!(StateSnapshot::decode(&bytes, PROVIDER, BINARY_SHA256, [0; 16]).is_err());
        assert!(StateSnapshot::decode(&bytes, "android", BINARY_SHA256, [1; 16]).is_err());
        let mut state: StateSnapshot = serde_json::from_slice(&bytes).unwrap();
        state.version = 2;
        assert!(
            StateSnapshot::decode(
                &serde_json::to_vec(&state).unwrap(),
                PROVIDER,
                BINARY_SHA256,
                [1; 16],
            )
            .is_err()
        );
        for path in [
            "/etc/passwd",
            "/var/db/fpsd/adi/../x",
            "/var/db/fpsd/adi//x",
        ] {
            let files = BTreeMap::from([(path.into(), "AA==".into())]);
            assert!(filesystem(&files).is_err());
        }
    }
    #[test]
    fn opaque_bytes_roundtrip_and_file_limits() {
        let mut files = BTreeMap::from([(
            "/var/db/fpsd/adi/test".into(),
            STANDARD.encode([0, 255, 0, 128]),
        )]);
        let encoded =
            StateSnapshot::encode(PROVIDER, BINARY_SHA256, [1; 16], [2; 16], files.clone())
                .unwrap();
        let parsed = StateSnapshot::decode(&encoded, PROVIDER, BINARY_SHA256, [1; 16]).unwrap();
        assert_eq!(
            filesystem(&parsed.files).unwrap().files["/var/db/fpsd/adi/test"],
            [0, 255, 0, 128]
        );
        files.insert(
            "/var/db/fpsd/adi/large".into(),
            STANDARD.encode(vec![0; 1024 * 1024 + 1]),
        );
        assert!(filesystem(&files).is_err());
    }
}
