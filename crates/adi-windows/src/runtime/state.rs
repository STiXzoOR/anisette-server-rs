use super::{BTreeMap, Binary, Error, Runtime};
use crate::{
    BINARY_SHA256,
    filesystem::{FileSystem, valid_path},
};
use anisette::protocol::StateSnapshot;
use base64::{Engine, engine::general_purpose::STANDARD};

const PROVIDER: &str = "windows-coreadi";

fn filesystem(files: &BTreeMap<String, String>) -> Result<FileSystem, Error> {
    if files.len() > 64 {
        return Err("too many state files".into());
    }
    let mut fs = FileSystem::default();
    for (path, encoded) in files {
        if !valid_path(path) {
            return Err("invalid state path".into());
        }
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|_| "invalid state encoding")?;
        let fd = fs
            .open(path.clone(), 0xc0000000, 0, 1)
            .map_err(|_| "invalid state file")?;
        fs.write(fd, &bytes)?;
        fs.close(fd);
    }
    Ok(fs)
}

impl Runtime {
    /// Exports the device state for later restoration.
    ///
    /// # Errors
    /// Returns an error if the runtime is poisoned, serialization fails, or the snapshot exceeds the state limit.
    pub fn export_state(&self, identifier: [u8; 16]) -> Result<Vec<u8>, Error> {
        if self.poisoned {
            return Err("failed runtime cannot export state".into());
        }
        let env = self.cpu.get_data();
        let files = env
            .fs
            .files
            .iter()
            .map(|(p, b)| (p.clone(), STANDARD.encode(b)))
            .collect();
        StateSnapshot::encode(PROVIDER, BINARY_SHA256, identifier, env.host_uuid, files)
            .map_err(Error::from)
    }
    /// Restores a device from a backend-specific state snapshot.
    ///
    /// # Errors
    /// Returns an error for invalid state size, identity, version, binary hash, or files, or if backend initialization fails.
    pub fn restore(binary: &Binary, identifier: [u8; 16], bytes: &[u8]) -> Result<Self, Error> {
        let state = StateSnapshot::decode(bytes, PROVIDER, BINARY_SHA256, identifier)?;
        let fs = filesystem(&state.files)?;
        let mut runtime = Self::new(binary)?;
        runtime.cpu.get_data_mut().host_uuid = state.host_uuid;
        runtime.cpu.get_data_mut().fs = fs;
        for path in state.files.keys() {
            let mut parent = path.as_str();
            while let Some((p, _)) = parent.rsplit_once('\\') {
                runtime.cpu.get_data_mut().directories.insert(p.into());
                parent = p;
            }
        }
        if runtime.initialize()? < 0 {
            return Err("restored Windows initialization failed".into());
        }
        Ok(runtime)
    }
}
#[cfg(test)]
mod tests {
    use super::{BINARY_SHA256, BTreeMap, PROVIDER, STANDARD, StateSnapshot, filesystem};
    use base64::Engine as _;
    #[test]
    fn state_rejects_identity_version_paths_and_size() {
        let encoded =
            StateSnapshot::encode(PROVIDER, BINARY_SHA256, [1; 16], [2; 16], BTreeMap::new())
                .unwrap();
        assert!(StateSnapshot::decode(&encoded, PROVIDER, BINARY_SHA256, [0; 16]).is_err());
        let mut state: StateSnapshot = serde_json::from_slice(&encoded).unwrap();
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
            "C:\\Windows\\x.pb",
            "c:\\programdata\\apple computer\\itunes\\adi\\..\\x.pb",
        ] {
            let files = BTreeMap::from([(path.into(), "AA==".into())]);
            assert!(filesystem(&files).is_err());
        }
        let files = BTreeMap::from([(
            format!("{}test.pb", crate::filesystem::ROOT),
            STANDARD.encode(vec![0; 1024 * 1024 + 1]),
        )]);
        assert!(filesystem(&files).is_err());
    }
}
