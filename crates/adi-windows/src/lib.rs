//! Fixed Windows CoreADI execution; V3 provisioning requires Apple routing.
mod filesystem;
mod pe;
mod runtime;
use anisette::protocol::verify_sha256;
pub use runtime::Runtime;
use std::{fs::File, io::Read, path::Path};

pub const BINARY_SHA256: &str = "1a75abd4891fab507a6fae3ab112f42a43da0c45b9e0fec7cbef45b946c7dcae";
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Invalid(&'static str),
    Io(std::io::Error),
    Unicorn(unicorn_engine::unicorn_const::uc_error),
    UnsupportedImport(String),
    Adi(i32),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(s) => f.write_str(s),
            Self::Io(error) => write!(f, "could not read ADI binary: {error}"),
            Self::Unicorn(error) => write!(f, "emulation failed: {error}"),
            Self::UnsupportedImport(name) => write!(f, "unsupported import: {name}"),
            Self::Adi(code) => write!(f, "ADI operation failed: {code}"),
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
pub struct Binary {
    image: pe::Image,
}
impl Binary {
    /// Loads and verifies the pinned Windows ADI binary.
    ///
    /// # Errors
    /// Returns an error if the file cannot be read, is not regular, exceeds the limit, fails hash verification, or has an unsupported PE image.
    pub fn open(path: &Path) -> Result<Self, Error> {
        let file = File::open(path).map_err(Error::Io)?;
        if !file.metadata().map_err(Error::Io)?.is_file() {
            return Err("binary must be regular".into());
        }
        let mut bytes = vec![];
        file.take(32 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(Error::Io)?;
        Self::verify(bytes)
    }
    fn verify(bytes: Vec<u8>) -> Result<Self, Error> {
        if bytes.len() > 32 * 1024 * 1024 || !verify_sha256(&bytes, BINARY_SHA256) {
            return Err("unrecognized Windows binary".into());
        }
        Ok(Self {
            image: pe::Image::parse(bytes)?,
        })
    }
    pub fn imports(&self) -> impl Iterator<Item = &str> {
        self.image.imports.iter().map(|(_, n)| n.as_str())
    }
}
#[cfg(test)]
mod tests {
    use super::{Binary, Error};
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
    fn unknown_build_is_rejected_before_parsing() {
        assert!(Binary::verify(vec![0; 32]).is_err());
    }
}
