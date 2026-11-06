# Compatibility

`anisette-server-rs` exposes the V1 and V3 Anisette endpoints across three Apple ADI backends (`android`, `macos`, and `windows`). Backend names refer to the guest Apple binary executed inside Unicorn, not the host operating system.

| Backend | Pinned guest binary | V1 (`GET /`) | V3 provisioning (`WS /v3/provisioning_session`) | `adi_pb` format |
| --- | --- | --- | --- | --- |
| `android` | Apple Music 4.9.6.1447 (`arm64-v8a` ELF) | Supported (saves Apple routing) | Standard V3 supported (`X-Apple-I-MD-RINFO` defaults to `17106176` if omitted) | Raw `adi.pb` bytes |
| `macos` | macOS 26.7.1 (`25G241`) `x86_64` `adid` | Supported (saves Apple routing) | Requires `X-Apple-I-MD-RINFO` in the finish message | `macos-adid` JSON envelope |
| `windows` | iTunes 12.13.12.3, `CoreADI.dll` 4.17.0.0 (`x86_64` PE) | Supported (saves Apple routing) | Requires `X-Apple-I-MD-RINFO` in the finish message | `windows-coreadi` JSON envelope |

State snapshots (`adi_pb` and `identity.json`) are backend-specific and cannot be shared across backends. Loaders verify exact SHA-256 hashes from [assets/manifests](../assets/manifests/README.md) before parsing.

## Host support

- **Linux (`x86_64` and `aarch64`) and macOS (`aarch64`)**: Full support for running the server and persisting private V1 state.
- **Windows hosts**: Offline unit tests build and pass, but Unix-mode private state storage (`0700`/`0600`) is not implemented on Windows hosts; run the Linux container via Docker Desktop or WSL2 on Windows.

All three backends execute inside isolated in-memory Unicorn VMs with virtual filesystems and do not access host Apple credentials, system registries, or Wine.

## V3 routing and HTTP profiles

- **V3 routing (`X-Apple-I-MD-RINFO`)**: Standard V3 finish-provisioning messages only send `ptm` and `tk`. While the Android ADI binary works with the standard `17106176` fallback when routing is omitted, the pinned macOS and Windows binaries return `rinfo = 0` unless the client forwards `X-Apple-I-MD-RINFO` from Apple's finish-provisioning response. Standard unmodified V3 clients should therefore use `--backend android`, whereas V1 (`GET /`) works with all three backends because the server performs the Apple HTTPS exchange directly.
- **HTTP client profiles**: The `android` and `windows` backends emit the Windows iCloud client profile (`X-MMe-Client-Info` / `User-Agent`), while `macos` emits the macOS `MacBookPro16,2` profile.

## Emulator bounds

Each backend implements only the ABI, syscalls, and imported library functions required by its pinned binary; unknown imports fail immediately. Every native Unicorn call is bounded to 5 seconds and 50 million instructions on non-executable stack/heap mappings (`W^X`), and a faulted VM instance is never reused.
