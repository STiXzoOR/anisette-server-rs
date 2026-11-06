//! Pinned Apple Music ARM64 libraries executed in a bounded Unicorn instance.
mod elf;
mod runtime;
use anisette::protocol::verify_sha256;
pub use runtime::Runtime;
use std::{fs::File, io::Read, path::Path};

pub const FILES: [(&str, &str); 2] = [
    (
        "libstoreservicescore.so",
        "bd335e6963ba5ae2f5babca613435ad56d2660a9aee37df35f876c458d105ce4",
    ),
    (
        "libCoreADI.so",
        "9d5b557cf3faf88c34c394ab2cd7fc1a3b4acf3aed51f28cd0bc093858107c22",
    ),
];
const MAX_BINARY: u64 = 32 * 1024 * 1024;
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Invalid(&'static str),
    Io(std::io::Error),
    Adi(i32),
    Unicorn(unicorn_engine::unicorn_const::uc_error),
    UnsupportedImport(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(s) => f.write_str(s),
            Self::Io(error) => write!(f, "could not read ADI binary: {error}"),
            Self::Adi(code) => write!(f, "ADI operation failed: {code}"),
            Self::Unicorn(error) => write!(f, "emulation failed: {error}"),
            Self::UnsupportedImport(name) => write!(f, "unsupported import: {name}"),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Unicorn(error) => Some(error),
            _ => None,
        }
    }
}
impl From<&'static str> for Error {
    fn from(s: &'static str) -> Self {
        Self::Invalid(s)
    }
}
impl From<anisette::memory::MemoryError> for Error {
    fn from(e: anisette::memory::MemoryError) -> Self {
        Self::Invalid(e.as_str())
    }
}
impl From<unicorn_engine::unicorn_const::uc_error> for Error {
    fn from(e: unicorn_engine::unicorn_const::uc_error) -> Self {
        Self::Unicorn(e)
    }
}
#[derive(Debug)]
pub struct Libraries {
    images: [elf::Image; 2],
}
impl Libraries {
    /// Loads and verifies the two pinned Android libraries from a directory.
    ///
    /// # Errors
    /// Returns an error if either library cannot be read, is not regular, exceeds the limit, fails hash verification, or has an unsupported ELF image.
    pub fn open(directory: &Path) -> Result<Self, Error> {
        let mut images = vec![];
        for (name, hash) in FILES {
            let file = File::open(directory.join(name)).map_err(Error::Io)?;
            if !file.metadata().map_err(Error::Io)?.is_file() {
                return Err("library must be regular file".into());
            }
            let mut bytes = vec![];
            file.take(MAX_BINARY + 1)
                .read_to_end(&mut bytes)
                .map_err(Error::Io)?;
            images.push(Self::verify(bytes, hash)?);
        }
        Ok(Self {
            images: images.try_into().map_err(|_| "missing Android library")?,
        })
    }
    fn verify(bytes: Vec<u8>, expected: &str) -> Result<elf::Image, Error> {
        if bytes.len() as u64 > MAX_BINARY {
            return Err("library too large".into());
        }
        if !verify_sha256(&bytes, expected) {
            return Err("unrecognized Android library hash".into());
        }
        elf::Image::parse(bytes)
    }
}
#[cfg(test)]
mod tests {
    use super::{Error, FILES, Libraries};
    #[test]
    fn errors_preserve_causes_and_diagnostics() {
        use std::error::Error as _;
        let error = Error::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "test permission denied",
        ));
        assert_eq!(
            error.source().unwrap().to_string(),
            "test permission denied"
        );
        assert!(error.to_string().contains("test permission denied"));
        assert!(Error::Adi(-45061).to_string().contains("-45061"));
        assert!(
            Error::UnsupportedImport("test_symbol".into())
                .to_string()
                .contains("test_symbol")
        );
        let error = Error::Unicorn(unicorn_engine::unicorn_const::uc_error::ARG);
        assert!(error.source().is_some());
    }
    #[test]
    fn rejects_unknown_build_before_parsing() {
        assert!(Libraries::verify(vec![0; 32], FILES[0].1).is_err());
    }
}
