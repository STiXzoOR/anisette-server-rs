//! Build-specific macOS 26 ADI loader and bounded execution runtime.
mod adi;
mod filesystem;
mod macho;
pub mod provisioning;
mod runtime;
mod state;
mod stubs;
mod xpc;
pub use runtime::Runtime;

use anisette::protocol::verify_sha256;
use std::{fs::File, io::Read, path::Path};

pub use macho::{Bind, Image, Segment};
pub const BINARY_SHA256: &str = "4b6b320f08edcf5d65ed1141399b76a8d9323fee2045c577c5d2f30f35a5dae1";
pub const MAX_BINARY_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Io(std::io::Error),
    TooLarge,
    UnknownBuild,
    Adi(i32),
    Execution(&'static str),
    UnsupportedImport(String),
    Unicorn(unicorn_engine::unicorn_const::uc_error),
    InvalidImage(&'static str),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Adi(code) => write!(f, "ADI operation failed: {code}"),
            Self::Execution(message) => f.write_str(message),
            Self::UnsupportedImport(name) => write!(f, "unsupported import: {name}"),
            Self::Unicorn(error) => write!(f, "emulation failed: {error}"),
            Self::Io(error) => write!(f, "could not read ADI binary: {error}"),
            Self::TooLarge => f.write_str("ADI binary exceeds size limit"),
            Self::UnknownBuild => f.write_str("unrecognized ADI binary SHA-256"),
            Self::InvalidImage(message) => f.write_str(message),
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

#[derive(Debug)]
pub struct Binary {
    bytes: Vec<u8>,
    image: Image,
}
impl Binary {
    /// Reads once, verifies those exact bytes, then parses. Execution must use these bytes too.
    ///
    /// # Errors
    /// Returns an error if the file cannot be read, is not regular, exceeds the limit, fails hash verification, or has an unsupported Mach-O image.
    pub fn open(path: &Path) -> Result<Self, Error> {
        let file = File::open(path).map_err(Error::Io)?;
        if !file.metadata().map_err(Error::Io)?.is_file() {
            return Err(Error::InvalidImage("binary must be a regular file"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_BINARY_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(Error::Io)?;
        Self::from_bytes(bytes)
    }
    /// Verifies the pinned binary hash before parsing its Mach-O image.
    ///
    /// # Errors
    /// Returns an error for oversized input, a hash mismatch, or an unsupported or invalid image.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, Error> {
        if bytes.len() as u64 > MAX_BINARY_BYTES {
            return Err(Error::TooLarge);
        }
        if !verify_sha256(&bytes, BINARY_SHA256) {
            return Err(Error::UnknownBuild);
        }
        let image = Image::parse(&bytes)?;
        Ok(Self { bytes, image })
    }
    #[must_use]
    pub fn image(&self) -> &Image {
        &self.image
    }
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl From<unicorn_engine::unicorn_const::uc_error> for Error {
    fn from(e: unicorn_engine::unicorn_const::uc_error) -> Self {
        Self::Unicorn(e)
    }
}

impl From<anisette::memory::MemoryError> for Error {
    fn from(e: anisette::memory::MemoryError) -> Self {
        Self::Execution(e.as_str())
    }
}

impl From<&'static str> for Error {
    fn from(message: &'static str) -> Self {
        Self::Execution(message)
    }
}

#[cfg(test)]
mod tests {
    use super::{Binary, Error, MAX_BINARY_BYTES};
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
    fn unknown_build_never_reaches_parser() {
        assert!(matches!(
            Binary::from_bytes(vec![0; 32]),
            Err(Error::UnknownBuild)
        ));
        assert!(matches!(
            Binary::from_bytes(vec![0; MAX_BINARY_BYTES as usize + 1]),
            Err(Error::TooLarge)
        ));
    }
}
