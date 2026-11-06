# Running and deployment

Start with the [README quick start](../README.md#quick-start-ghcr-container), or see the configuration details below. For backend selection and client compatibility, see [Compatibility](compatibility.md).

## Docker (GHCR)

CI publishes multi-architecture images (`linux/amd64` and `linux/arm64`) to GitHub Container Registry (`ghcr.io/fdkevin0/anisette-server-rs`). Both `:latest` and `:main` track successful `main` branch builds; version tags such as `:0.1.0` pin releases. Each image bundles all three verified Apple ADI backends (`android`, `macos`, and `windows`).

```sh
docker volume create anisette-macos-data
docker run --rm --name anisette-macos -p 127.0.0.1:6969:6969 \
  --mount type=volume,src=anisette-macos-data,dst=/data \
  ghcr.io/fdkevin0/anisette-server-rs:latest --backend macos
```

Use `--backend android`, `--backend macos`, or `--backend windows` to select a backend. Each running container serves one backend and must use its own dedicated state volume (`/data`). Do not share a state volume across backends or concurrent container instances.

## Running locally

Rust 1.95.0 is pinned via `rust-toolchain.toml`. Install CMake, Ninja (`ninja-build`), a C/C++ compiler, and `libclang`, then [prepare the Apple binaries](../assets/README.md).

Ensure the parent directory of `--state-dir` exists before starting the server (for `--state-dir ./data`, `.` already exists):

```sh
cargo run --release -p anisette-server -- --backend android \
  --binary ./assets/local/android-4.9.6 --state-dir ./data
```

For macOS ADI, pass `--backend macos --binary ./assets/local/CoreADI-26.x86_64`; for Windows ADI, pass `--backend windows --binary ./assets/local/CoreADI.dll`. Private state storage requires a Unix host (`0700` directory and `0600` file permissions).

## Configuration

| Setting | CLI / environment | Local default | Docker default |
| --- | --- | --- | --- |
| Backend | `--backend` | `android` | `android` |
| Listener | `--listen` / `ANISETTE_LISTEN` | `127.0.0.1:6969` | `0.0.0.0:6969` |
| State directory | `--state-dir` / `ANISETTE_STATE_DIR` | `data` | `/data` |
| Binary override | `--binary` | Backend path below | Backend path below |

CLI arguments override environment variables. Default `--binary` paths are `/assets` for Android, `/assets/CoreADI-26.x86_64` for macOS, and `/assets/CoreADI.dll` for Windows.

Keep the published host port bound to `127.0.0.1` for local use. Public deployments should run behind a reverse proxy with TLS and access control.

## Device state and restart

On startup (and on V1 requests if not yet provisioned), the server provisions an identity with Apple and atomically writes `identity.json` before serving V1 headers. Subsequent restarts restore the saved identity without contacting Apple; generating OTPs for a provisioned device requires no network access. Corrupted or mismatched state files fail startup rather than silently resetting the device identity.

The Docker container runs as UID/GID `10001`. Named Docker volumes automatically inherit `/data` ownership (`10001:10001`, mode `0700`); host bind mounts must be owned by UID `10001` with mode `0700`.

### Storage and shutdown invariants

- The parent directory of `--state-dir` must already exist.
- State directories enforce mode `0700` and state files enforce mode `0600`; wrong ownership, symlinks, and multiple hard links are rejected.
- An exclusive lock (`identity.lock`) prevents concurrent processes from sharing a state directory.
- State records validate `version`, `backend`, `identifier`, `adi_pb`, and full header metadata (`device_id`, 64-hex `local_user_id`, `serial`, `client_info`, and `user_agent`).
- Saves write to a temporary file, `fsync`, rename atomically, and `fsync` the directory.
- `SIGINT`/`SIGTERM` stops accepting new requests, closes active WebSockets, waits for in-flight execution and state saves to complete, and releases `identity.lock`.

## Proxies and offline checks

Provisioning respects `HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY`, and `NO_PROXY` (and lowercase variants). When running in Docker, forward proxy variables explicitly (`docker run -e HTTPS_PROXY -e NO_PROXY ...`) and ensure the proxy address is reachable from inside the container namespace.

To verify bundled binary loading without network access or state initialization:

```sh
docker run --rm --network none ghcr.io/fdkevin0/anisette-server-rs:latest --backend android --check
```

Repeat with `--backend macos` or `--backend windows`.

### Provisioning TLS settings

Provisioning uses `reqwest` with `rustls`, platform root certificates, and the bundled Apple Inc. Root CA (`crates/anisette/apple-root-ca.pem`). Redirects are disabled, provisioning URLs are restricted to `https://gsa.apple.com`, and requests enforce a 10-second connect timeout, 20-second total timeout, and 1 MiB response body limit.

## Local image builds

```sh
docker build -t anisette-server-rs:local .
```

The multi-stage Dockerfile downloads and verifies all three pinned backends by SHA-256 during build and copies only the verified libraries and compiled server binary into the runtime image. If an upstream download URL (such as the unversioned Android `applemusic.apk` endpoint) serves a changed file, `docker build` fails closed at the verification step while pre-built GHCR images continue to work normally. See [Asset preparation](../assets/README.md) for upstream download URLs and hashes.
