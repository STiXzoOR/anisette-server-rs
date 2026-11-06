# Development and validation

Use this guide to build, lint, test, and verify the workspace. For local prerequisites and binary preparation, see [Running locally](operations.md#running-locally) and [Asset preparation](../assets/README.md).

## Prerequisites

- Rust `1.95.0` (pinned in `rust-toolchain.toml`)
- CMake, Ninja (`ninja-build`), a C/C++ compiler, and `libclang` (required to compile `unicorn-engine-sys`)
- `7z` (`p7zip-full`) when extracting the Windows `CoreADI.dll` from the iTunes installer locally

## Offline checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features --locked
```

Offline tests run without Apple binaries or external network access. They require a host environment that allows Unicorn JIT mappings and loopback sockets.

The offline suite covers:
- Base64/JSON protocol parsing, bounds, and error redaction
- Provisioning session state machine ordering and cancellation
- HTTP method routing (`GET /`, `GET, HEAD /v3/client_info`, `POST /v3/get_headers`), body timeouts, and WebSocket `Origin` rejection
- WebSocket handshake timeouts, disconnection, and graceful shutdown
- Private state directory permissions (`0700`/`0600`), flock exclusion, and atomic saves
- ELF, Mach-O, and PE loader bounds and segment overlap checks
- Guest heap allocation, W^X memory protection, TLS slot reuse, and instruction/time limits

## Real-backend validation

With all three pinned Apple binaries prepared locally, run the full live test suite against Apple's provisioning endpoints:

```sh
ANISETTE_ANDROID_LIBS=./assets/local/android-4.9.6 \
ANISETTE_MACOS_BINARY=./assets/local/CoreADI-26.x86_64 \
ANISETTE_WINDOWS_BINARY=./assets/local/CoreADI.dll \
  sh scripts/verify-backends.sh
```

To run a single backend's live integration test:

```sh
ANISETTE_ANDROID_LIBS=./assets/local/android-4.9.6 \
  cargo test -p anisette-server live_android_http_restart_and_standard_v3 -- --ignored
ANISETTE_MACOS_BINARY=./assets/local/CoreADI-26.x86_64 \
  cargo test -p anisette-server live_http_restart_v3_restore_and_extended_provisioning -- --ignored
ANISETTE_WINDOWS_BINARY=./assets/local/CoreADI.dll \
  cargo test -p anisette-server live_windows_http_restart_and_extended_v3 -- --ignored
```

### Offline binary load checks

To verify binary loading and VM initialization locally without contacting Apple:

```sh
cargo run -p adi-android --example boot -- ./assets/local/android-4.9.6
cargo run -p adi-macos --example inspect -- ./assets/local/CoreADI-26.x86_64
ANISETTE_MACOS_BINARY=./assets/local/CoreADI-26.x86_64 \
  cargo test -p adi-macos --test binary -- --ignored
ANISETTE_WINDOWS_BINARY=./assets/local/CoreADI.dll \
  cargo test -p adi-windows --test binary -- --ignored
```

## CI and container publishing

GitHub Actions (`.github/workflows/ci.yml`) runs formatting, Clippy, offline unit/integration tests, and rustdoc checks. On pushes to `main` and version tags (`v*`), native `linux/amd64` and `linux/arm64` runners build the Docker image (which verifies `--check` across all three backends during build), push per-architecture digests, and publish the multi-arch manifest to `ghcr.io/fdkevin0/anisette-server-rs`.

## Workspace layout

| Crate | Responsibility |
| --- | --- |
| `anisette` | Shared V1/V3 protocol types, ADI wire constants, provisioning session state machine, Apple HTTPS client, and guest heap allocator |
| `adi-android` | Pinned Android ELF loader, ARM64 Unicorn runtime, ADI operations, and APK importer |
| `adi-macos` | Pinned Mach-O loader, `x86_64` Unicorn runtime, CoreFoundation/XPC stubs, and macOS state snapshot |
| `adi-windows` | Pinned PE loader, `x86_64` Unicorn runtime, Win32 API subset, Windows state snapshot, and iTunes installer extractor |
| `anisette-server` | CLI, Hyper HTTP/WebSocket server, bounded worker pools, and private V1 identity persistence |

## Lint policy

Workspace-wide lints are configured in the root `Cargo.toml` under `[workspace.lints]` and inherited by every crate (`[lints] workspace = true`). They enforce `unsafe_op_in_unsafe_fn`, `undocumented_unsafe_blocks`, `missing_errors_doc`, `wildcard_imports`, `dbg_macro`, `todo`, and rustdoc link checks.
