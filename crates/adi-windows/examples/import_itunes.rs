//! Explicit offline extraction; 7z is a preparation tool, never a service dependency.
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
const INSTALLER_SIZE: u64 = 202_558_432;
const INSTALLER_HASH: &str = "221a24203f6182b7dc0e18055b1e9338a0c3c992c7fefe6b3be0460caf29db4d";
const MSI_SIZE: u64 = 174_817_280;
const MSI_HASH: &str = "b28e7b53865c5dc7c3fd48d707ee5c95dc2f51dd2e696a023bbb7c50a449f6f3";
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn verified_copy(source: &Path, output: &mut File, size: u64, hash: &str) -> Result<()> {
    let input = File::open(source)?;
    if !input.metadata()?.is_file() || input.metadata()?.len() != size {
        return Err("unrecognized asset size or type".into());
    }
    let mut input = input.take(size + 1);
    let mut digest = Sha256::new();
    let mut count = 0u64;
    let mut buffer = [0; 65536];
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > size {
            return Err("asset exceeds size limit".into());
        }
        digest.update(&buffer[..n]);
        output.write_all(&buffer[..n])?;
    }
    let actual = digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    if count != size || actual != hash {
        return Err("unrecognized asset hash".into());
    }
    Ok(())
}
fn extract(archive: &Path, entry: &str, output: &Path, limit: u64) -> Result<()> {
    let file = File::create_new(output)?;
    let mut child = Command::new("7z")
        .arg("e")
        .arg("-so")
        .arg(archive)
        .arg(entry)
        .stdin(Stdio::null())
        .stdout(Stdio::from(file.try_clone()?))
        .stderr(Stdio::null())
        .spawn()?;
    let started = Instant::now();
    let result = (|| -> Result<()> {
        loop {
            if file.metadata()?.len() > limit {
                return Err("extracted asset exceeds size limit".into());
            }
            if let Some(status) = child.try_wait()? {
                if !status.success() {
                    return Err("7z extraction failed".into());
                }
                if file.metadata()?.len() > limit {
                    return Err("extracted asset exceeds size limit".into());
                }
                return Ok(());
            }
            if started.elapsed() > Duration::from_secs(30) {
                return Err("7z extraction timed out".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}
fn import(installer: &Path, destination: &Path) -> Result<()> {
    if destination.try_exists()? || destination.symlink_metadata().is_ok() {
        return Err("destination must not exist".into());
    }
    let temp = tempfile::tempdir()?;
    let verified = temp.path().join("verified.exe");
    verified_copy(
        installer,
        &mut File::create_new(&verified)?,
        INSTALLER_SIZE,
        INSTALLER_HASH,
    )?;
    let msi = temp.path().join("iTunes64.msi");
    extract(&verified, "iTunes64.msi", &msi, MSI_SIZE)?;
    let verified_msi = temp.path().join("verified.msi");
    verified_copy(
        &msi,
        &mut File::create_new(&verified_msi)?,
        MSI_SIZE,
        MSI_HASH,
    )?;
    let dll = temp.path().join("CoreADI.dll");
    extract(
        &verified_msi,
        "fil69B791D2E52CAEC1073A4E4246665479",
        &dll,
        1_720_672,
    )?;
    adi_windows::Binary::open(&dll)?;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut output = tempfile::NamedTempFile::new_in(parent)?;
    verified_copy(
        &dll,
        output.as_file_mut(),
        1_720_672,
        adi_windows::BINARY_SHA256,
    )?;
    output.as_file().sync_all()?;
    output.persist_noclobber(destination).map_err(|e| e.error)?;
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err(
            "usage: import_itunes VERIFIED_INSTALLER.exe NEW_CoreADI.dll (requires 7z)".into(),
        );
    }
    import(Path::new(&args[0]), Path::new(&args[1]))?;
    println!("Verified Windows CoreADI imported; no provisioning performed");
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::import;
    #[test]
    fn rejects_bad_input_without_extraction_or_overwrite() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input.exe");
        let output = root.path().join("CoreADI.dll");
        std::fs::write(&input, b"invalid").unwrap();
        assert!(import(&input, &output).is_err());
        assert!(!output.exists());
        std::fs::write(&output, b"original").unwrap();
        assert!(import(&input, &output).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), b"original");
    }
}
