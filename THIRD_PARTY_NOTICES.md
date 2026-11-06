# Third-party notices

Repository source code is licensed under `GPL-2.0-only` (see [LICENSE](LICENSE)). No Apple binaries are committed to the source repository; Docker builds download and verify pinned Apple binaries at build time, and those files remain subject to their own distribution terms. Full Rust dependency licenses are listed in [docs/licenses.md](docs/licenses.md), and upstream commit references are recorded in [docs/upstream.md](docs/upstream.md).

## macOS backend (`crates/adi-macos` and `crates/anisette/src/memory.rs`)

- `crates/adi-macos/src/macho.rs` is adapted from [sacca97/os-findmy-e](https://github.com/sacca97/os-findmy-e/tree/d4d2a832baf47d1cf43d989879950ac2aec45f47) (`native/adi-core/src/macho.rs`), Copyright (c) 2026 sacca, licensed under `GPL-2.0-only` (see `crates/adi-macos/LICENSE`).
- The guest heap allocator (`crates/anisette/src/memory.rs`), CoreADI dispatcher framing (`crates/adi-macos/src/adi.rs`), and macOS daemon/CoreFoundation/XPC stub models (`crates/adi-macos/src/{filesystem,runtime,stubs,xpc}.rs`) are adapted from `os-findmy-e`'s `macOS-ADI-emulator` (`harness.py`, `adi.py`, and `stubs.py`), Copyright (c) 2026 sacca, `GPL-2.0-only`.

## Android backend (`crates/adi-android`)

- `crates/adi-android/src/runtime.rs` adapts the ARM64 Unicorn memory layout and symbol hook behavior from [malmeloo/Anisette.py](https://github.com/malmeloo/Anisette.py/tree/627c9c663062bd872182b653113759c9d66b7e13) (`src/anisette/vm/_vm.py` and `_hooks.py`). Its MIT copyright and permission notice is preserved in `crates/adi-android/LICENSE-MIT-Anisette.py`.
- The ADI dispatcher framing and shared allocator follow the `GPL-2.0-only` `os-findmy-e` reference; the combined crate is licensed under `GPL-2.0-only` (`crates/adi-android/LICENSE`).

## Windows backend (`crates/adi-windows`)

- `crates/adi-windows` uses a locally authored PE loader and bounded Win64 API shim together with the `GPL-2.0-only` `os-findmy-e` ADI dispatcher framing and shared guest allocator (`crates/adi-windows/LICENSE`).

## Protocol references

- V1 and V3 HTTP/WebSocket semantics follow [Dadoum/anisette-v3-server](https://github.com/Dadoum/anisette-v3-server/tree/684895737aaf1a1c4586fcd5b67509ffd500a36c) (`source/app.d`, protocol reference only; no code imported) and [SideStore/apple-private-apis](https://github.com/SideStore/apple-private-apis/tree/03beb1aa42991ccdad6214dee77e72282bef461f) (`omnisette/src/remote_anisette_v3.rs`, MPL-2.0, protocol reference only).

## Apple public root certificate

- `crates/anisette/apple-root-ca.pem` is the public Apple Inc. Root CA certificate from [Apple PKI](https://www.apple.com/certificateauthority/) used for TLS verification of `https://gsa.apple.com`.
