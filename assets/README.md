# Apple binary assets

Use this guide when running `anisette-server` directly on the host without Docker. [Published GHCR images](../README.md#quick-start-ghcr-container) already bundle all three verified backends.

Apple ADI binaries are not committed to Git (`assets/local/` is ignored). Loaders verify exact SHA-256 digests before parsing; see [assets/manifests](manifests/README.md) for the pinned versions and hashes.

> [!IMPORTANT]
> **What happens if upstream binary files change**:
> - **Fail-closed verification**: All importers (`import_apk`, `import_itunes`), the `Dockerfile`, and the runtime loaders (`Libraries::open` / `Binary::open`) verify exact file sizes and SHA-256 digests *before* parsing ELF, Mach-O, or PE structures.
> - **Mutable URLs (`applemusic.apk` and ShareBin)**: Because Apple's Android APK URL (`https://apps.mzstatic.com/content/android-apple-music-apk/applemusic.apk`) is unversioned, if Apple publishes a newer APK at that URL, new `docker build` or `import_apk` runs will stop with a size/SHA-256 mismatch rather than executing an unverified binary or overwriting existing prepared files. Already-published GHCR images (`ghcr.io/fdkevin0/anisette-server-rs:latest` or release tags) bundle the verified libraries inside the image and remain unaffected.
> - **Upgrading to a new binary build**: Because each backend binds to build-specific symbols, entry points, or obfuscated export names (and macOS/Windows `StateSnapshot` envelopes pin `binary_sha256`), supporting a newer Apple binary requires updating the pinned hashes, symbol/entry constants, and live-test verification together in code.

## Android (`applemusic.apk`)

Pinned build: Apple Music **4.9.6.1447** (`arm64-v8a`), defined in [manifests/android-4.9.6.json](manifests/android-4.9.6.json).

```sh
mkdir -p assets/local
curl --fail --location --proto '=https' --proto-redir '=https' \
  --max-time 180 --max-filesize 150000000 \
  --output assets/local/applemusic.apk.part \
  https://apps.mzstatic.com/content/android-apple-music-apk/applemusic.apk
cargo run -p adi-android --example import_apk -- \
  assets/local/applemusic.apk.part assets/local/android-4.9.6
```

`import_apk` verifies the APK's SHA-256, extracts `libstoreservicescore.so` and `libCoreADI.so`, verifies their hashes and ELF structure, and atomically writes the output directory.

## macOS (`CoreADI-26.x86_64`)

Pinned build: macOS 26.7.1 (`25G241`) `x86_64` `adid` slice, SHA-256 `4b6b320f08edcf5d65ed1141399b76a8d9323fee2045c577c5d2f30f35a5dae1` ([manifests/macos-26.json](manifests/macos-26.json)), referenced in [malmeloo/Anisette.py#6](https://github.com/malmeloo/Anisette.py/pull/6):

```sh
mkdir -p assets/local
curl --disable --fail --location --proto '=https' --proto-redir '=https' \
  --connect-timeout 30 --max-time 120 --max-filesize 3677696 --data '' \
  --output assets/local/CoreADI-26.x86_64 https://sharebin.eu/files/CoreADI.x86_64
cargo run -p adi-macos --example inspect -- assets/local/CoreADI-26.x86_64
```

## Windows (`CoreADI.dll`)

Pinned build: iTunes **12.13.12.3** `x86_64` `CoreADI.dll` 4.17.0.0 ([manifests/windows-itunes-12.13.12.3.json](manifests/windows-itunes-12.13.12.3.json)).

Install `7z` (`p7zip-full` on Debian/Ubuntu), download the official iTunes installer, and run `import_itunes`:

```sh
mkdir -p assets/local
curl --fail --location --proto '=https' --proto-redir '=https' \
  --max-time 180 --max-filesize 210000000 \
  --output assets/local/iTunes64Setup.exe.part \
  https://secure-appldnld.apple.com/itunes12/142-28289-20261026-6acff6c8-72eb-4045-84eb-fd35239ac645/iTunes64Setup.exe
cargo run -p adi-windows --example import_itunes -- \
  assets/local/iTunes64Setup.exe.part assets/local/CoreADI.dll
```

`import_itunes` verifies the installer's SHA-256, extracts `iTunes64.msi` and `CoreADI.dll` in a temporary directory, verifies both hashes, and writes `assets/local/CoreADI.dll`.
