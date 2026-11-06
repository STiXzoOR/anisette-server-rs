# Sources and provenance

The table below records the upstream projects referenced or adapted in `anisette-server-rs`. Full copyright and license notices are in [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md).

| Upstream project | Pinned commit | Role and license |
| --- | --- | --- |
| [Dadoum/anisette-v3-server](https://github.com/Dadoum/anisette-v3-server/tree/684895737aaf1a1c4586fcd5b67509ffd500a36c) | `684895737aaf1a1c4586fcd5b67509ffd500a36c` | V1/V3 HTTP and WebSocket protocol specification (`source/app.d`); protocol reference only |
| [SideStore/apple-private-apis](https://github.com/SideStore/apple-private-apis/tree/03beb1aa42991ccdad6214dee77e72282bef461f) | `03beb1aa42991ccdad6214dee77e72282bef461f` | MPL-2.0; reference for `omnisette` V3 message encoding and opaque `adi_pb` client storage |
| [malmeloo/Anisette.py](https://github.com/malmeloo/Anisette.py/tree/627c9c663062bd872182b653113759c9d66b7e13) | `627c9c663062bd872182b653113759c9d66b7e13` | MIT; Android ARM64 VM layout, symbol hooks, and RemoteADI client reference |
| [sacca97/os-findmy-e](https://github.com/sacca97/os-findmy-e/tree/d4d2a832baf47d1cf43d989879950ac2aec45f47) | `d4d2a832baf47d1cf43d989879950ac2aec45f47` | GPL-2.0-only; Mach-O parser, guest heap allocator, CoreADI dispatcher framing, and macOS `adid` CF/XPC stub models |

## Guest binaries and HTTP profiles

- **Guest binaries**: Expected sizes, versions, and SHA-256 digests are recorded in [assets/manifests](../assets/manifests/README.md). No Apple binaries are committed to Git; Docker builds fetch and verify them by SHA-256 at image build time.
- **HTTP profiles**: The Windows client profile (`X-MMe-Client-Info` and `User-Agent`) used by the Android and Windows backends follows the header format in [alex-mextner/passwords-app (`53c74a89`)](https://github.com/alex-mextner/passwords-app/commit/53c74a89a3b64d8e02eab5cc8b297dd22f5634cd).

## Platform and TLS references

- **Windows x64 ABI**: The `adi-windows` PE loader and Win32 shim follow the [Microsoft x64 calling convention](https://learn.microsoft.com/en-us/cpp/build/x64-calling-convention?view=msvc-170) and [MultiByteToWideChar](https://learn.microsoft.com/en-us/windows/win32/api/stringapiset/nf-stringapiset-multibytetowidechar) / `WideCharToMultiByte` semantics needed by `CoreADI.dll`.
- **Apple Root CA**: `crates/anisette/apple-root-ca.pem` is the public [Apple Inc. Root certificate](https://www.apple.com/appleca/AppleIncRootCertificate.cer) from [Apple PKI](https://www.apple.com/certificateauthority/) (DER SHA-256 `b0b1730ecbc7ff4505142c49f1295e6eda6bcaed7e2c68c5be91b5a11001f024`), used alongside system trust roots when verifying `https://gsa.apple.com`.
- **Rust dependencies**: See [docs/licenses.md](licenses.md) for the workspace dependency and license inventory.
