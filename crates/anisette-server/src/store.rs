use anisette::protocol::{MAX_STATE_BYTES, decode_binary};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::fs;
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const MAX_RECORD: u64 = 5 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    backend: String,
    identifier: [u8; 16],
    adi_pb: String,
    headers: HeaderIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeaderIdentity {
    pub device_id: String,
    pub local_user_id: String,
    pub serial: String,
    pub client_info: String,
    pub user_agent: String,
}
impl HeaderIdentity {
    pub fn validate(&self) -> io::Result<()> {
        anisette::protocol::Identifier::from_uuid(&self.device_id)
            .map_err(|_| invalid("invalid saved device UUID"))?;
        let hex = |value: &[u8]| value.len() == 64 && value.iter().all(u8::is_ascii_hexdigit);
        if !hex(self.local_user_id.as_bytes()) {
            return Err(invalid("invalid saved local UUID header"));
        }
        for (value, max) in [
            (&self.serial, 64),
            (&self.client_info, 512),
            (&self.user_agent, 512),
        ] {
            if value.is_empty()
                || value.len() > max
                || !value.bytes().all(|b| (32..127).contains(&b))
            {
                return Err(invalid("invalid saved identity header"));
            }
        }
        Ok(())
    }
}
pub type LoadedState = ([u8; 16], Vec<u8>, HeaderIdentity);

#[derive(Debug)]
pub struct Store {
    directory: PathBuf,
    backend: &'static str,
    _lock: File,
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::other(message)
}

impl Store {
    pub fn open(path: &Path, backend: &'static str) -> io::Result<Self> {
        #[cfg(not(unix))]
        {
            let _ = (path, backend);
            return Err(invalid(
                "private state storage is not implemented on this host",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
            match fs::DirBuilder::new().mode(0o700).create(path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
            let metadata = fs::symlink_metadata(path)?;
            // The private directory prevents another user replacing files between checks.
            if !metadata.is_dir()
                || metadata.mode() & 0o077 != 0
                || metadata.uid() != {
                    // SAFETY: geteuid takes no arguments and has no caller preconditions.
                    unsafe { libc::geteuid() }
                }
            {
                return Err(invalid(
                    "state directory must be owned by this user and mode 0700",
                ));
            }
            let directory = path.canonicalize()?;
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(directory.join("identity.lock"))?;
            Self::check_file(&lock)?;
            lock.try_lock()
                .map_err(|_| invalid("state directory already in use or cannot be locked"))?;
            Ok(Self {
                directory,
                backend,
                _lock: lock,
            })
        }
    }

    fn check_file(file: &File) -> io::Result<()> {
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(invalid("state must be a regular file"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.mode() & 0o077 != 0
                || metadata.uid() != {
                    // SAFETY: geteuid takes no arguments and has no caller preconditions.
                    unsafe { libc::geteuid() }
                }
                || metadata.nlink() != 1
            {
                return Err(invalid(
                    "state file must be private, owned, and have one link",
                ));
            }
        }
        Ok(())
    }

    pub fn load(&self) -> io::Result<Option<LoadedState>> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = match options.open(self.directory.join("identity.json")) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        Self::check_file(&file)?;
        let mut bytes = vec![];
        file.take(MAX_RECORD + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_RECORD {
            return Err(invalid("state too large"));
        }
        let record: Record =
            serde_json::from_slice(&bytes).map_err(|_| invalid("invalid state record"))?;
        if record.version != 1 || record.backend != self.backend {
            return Err(invalid("state backend/version mismatch"));
        }
        let bytes = decode_binary(&record.adi_pb, MAX_STATE_BYTES)
            .map_err(|_| invalid("invalid state bytes"))?;
        record.headers.validate()?;
        Ok(Some((record.identifier, bytes, record.headers)))
    }

    pub fn save(
        &self,
        identifier: [u8; 16],
        bytes: &[u8],
        headers: &HeaderIdentity,
    ) -> io::Result<()> {
        if bytes.is_empty() || bytes.len() > MAX_STATE_BYTES {
            return Err(invalid("invalid state size"));
        }
        headers.validate()?;
        let record = Record {
            version: 1,
            backend: self.backend.into(),
            identifier,
            adi_pb: STANDARD.encode(bytes),
            headers: headers.clone(),
        };
        let mut file = tempfile::NamedTempFile::new_in(&self.directory)?;
        serde_json::to_writer(&mut file, &record)
            .map_err(|_| invalid("state serialization failed"))?;
        file.flush()?;
        file.as_file().sync_all()?;
        file.persist(self.directory.join("identity.json"))
            .map_err(|e| e.error)?;
        #[cfg(unix)]
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{HeaderIdentity, STANDARD, Store, fs};
    use base64::Engine as _;
    use std::os::unix::fs::{PermissionsExt, symlink};
    fn headers() -> HeaderIdentity {
        HeaderIdentity {
            device_id: anisette::protocol::Identifier::from_bytes([7; 16]).uuid(),
            local_user_id: anisette::provisioning::local_user_id([7; 16]),
            serial: "0".into(),
            client_info: "<Mac> <macOS;26>".into(),
            user_agent: "akd/1.0".into(),
        }
    }

    #[test]
    fn rejects_incomplete_records_without_modifying_state() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("device");
        let store = Store::open(&directory, "macos").unwrap();
        store.save([7; 16], b"state", &headers()).unwrap();
        let path = directory.join("identity.json");
        let original: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        for missing in [true, false] {
            let mut record = original.clone();
            if missing {
                record.as_object_mut().unwrap().remove("headers");
            } else {
                record["headers"] = serde_json::Value::Null;
            }
            let bytes = serde_json::to_vec(&record).unwrap();
            fs::write(&path, &bytes).unwrap();
            assert!(store.load().is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn invalid_identity_is_rejected_on_load_and_save() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("device");
        let store = Store::open(&directory, "macos").unwrap();
        store.save([7; 16], b"state", &headers()).unwrap();
        let path = directory.join("identity.json");
        let original = fs::read(&path).unwrap();
        for invalid in [
            HeaderIdentity {
                device_id: "invalid".into(),
                ..headers()
            },
            HeaderIdentity {
                local_user_id: STANDARD.encode("AB".repeat(32)),
                ..headers()
            },
            HeaderIdentity {
                user_agent: "bad\r\nheader".into(),
                ..headers()
            },
        ] {
            assert!(store.save([7; 16], b"replacement", &invalid).is_err());
            assert_eq!(fs::read(&path).unwrap(), original);
            let mut record: serde_json::Value = serde_json::from_slice(&original).unwrap();
            record["headers"] = serde_json::to_value(invalid).unwrap();
            fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
            assert!(store.load().is_err());
            fs::write(&path, &original).unwrap();
        }
    }

    #[test]
    fn locked_private_atomic_state_survives_reopen_and_rejects_corruption() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("device");
        let store = Store::open(&path, "macos").unwrap();
        assert!(store.load().unwrap().is_none());
        assert!(Store::open(&path, "macos").is_err());
        store.save([7; 16], b"first", &headers()).unwrap();
        store.save([7; 16], b"second", &headers()).unwrap();
        assert_eq!(
            fs::metadata(path.join("identity.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(store.save([7; 16], b"", &headers()).is_err());
        drop(store);
        let store = Store::open(&path, "macos").unwrap();
        assert_eq!(
            store.load().unwrap().unwrap(),
            ([7; 16], b"second".to_vec(), headers())
        );
        fs::write(path.join("identity.json"), b"invalid").unwrap();
        assert!(store.load().is_err());
        fs::remove_file(path.join("identity.json")).unwrap();
        symlink(root.path().join("absent"), path.join("identity.json")).unwrap();
        assert!(store.load().is_err());
    }
    #[test]
    fn rejects_public_or_symlink_directory_and_nonprivate_state() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("device");
        let store = Store::open(&path, "macos").unwrap();
        store.save([0; 16], b"test", &headers()).unwrap();
        fs::set_permissions(
            path.join("identity.json"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert!(store.load().is_err());
        drop(store);
        symlink(&path, root.path().join("link")).unwrap();
        assert!(Store::open(&root.path().join("link"), "macos").is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(Store::open(&path, "macos").is_err());
    }
}
