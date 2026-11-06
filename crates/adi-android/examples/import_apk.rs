//! Explicit offline import of the pinned Apple Music APK; never run at service startup.
use adi_android::{FILES, Libraries};
use anisette::protocol::verify_sha256;
use std::{
    fs,
    io::{Cursor, Read, Write},
    path::Path,
};

const APK_HASH: &str = "9aab4e3bfd44b509dd657b030d46df536c5592512c256fc59012cce47a2e9c9c";
const APK_SIZE: u64 = 142_139_820;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn import(apk: &Path, destination: &Path) -> Result<()> {
    if destination.try_exists()? || fs::symlink_metadata(destination).is_ok() {
        return Err("destination must not exist".into());
    }
    let file = fs::File::open(apk)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() != APK_SIZE {
        return Err("unrecognized APK size or file type".into());
    }
    let mut bytes = Vec::new();
    file.take(APK_SIZE + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 != APK_SIZE || !verify_sha256(&bytes, APK_HASH) {
        return Err("unrecognized APK hash".into());
    }
    // Parse the verified in-memory bytes, so a replaced source cannot change extraction.
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = tempfile::tempdir_in(parent)?;
    for (name, expected) in FILES {
        let entry = archive.by_name(&format!("lib/arm64-v8a/{name}"))?;
        let mut bytes = Vec::new();
        entry.take(32 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 32 * 1024 * 1024 || !verify_sha256(&bytes, expected) {
            return Err("unrecognized extracted library".into());
        }
        let mut output = fs::File::create_new(temporary.path().join(name))?;
        output.write_all(&bytes)?;
        output.sync_all()?;
    }
    Libraries::open(temporary.path())?;
    // Reserve the destination before publishing; competing imports cannot overwrite it.
    fs::create_dir(destination)?;
    if let Err(error) = fs::rename(temporary.path(), destination) {
        let _ = fs::remove_dir(destination);
        return Err(error.into());
    }
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: import_apk APPLEMUSIC.apk NEW_DIRECTORY".into());
    }
    import(Path::new(&args[0]), Path::new(&args[1]))?;
    println!("Verified Android ARM64 libraries imported; no provisioning performed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{fs, import};
    #[test]
    fn rejected_input_leaves_no_output_and_existing_directory_untouched() {
        let root = tempfile::tempdir().unwrap();
        let apk = root.path().join("wrong.apk");
        fs::write(&apk, b"not an APK").unwrap();
        let output = root.path().join("out");
        assert!(import(&apk, &output).is_err());
        assert!(!output.exists());
        fs::create_dir(&output).unwrap();
        fs::write(output.join("keep"), b"original").unwrap();
        assert!(import(&apk, &output).is_err());
        assert_eq!(fs::read(output.join("keep")).unwrap(), b"original");
    }
}
