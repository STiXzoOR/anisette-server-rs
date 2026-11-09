# anisette-server-rs

Anisette V1/V3 server implemented in Rust. It executes pinned Apple ADI binaries (Android, macOS, and Windows) inside Unicorn CPU emulators to generate Anisette headers for clients such as `ipatool`, `AltServer`, and `SideStore` (`omnisette`).

## Quick start (GHCR container)

Published multi-architecture images (`linux/amd64` and `linux/arm64`) are available on GitHub Container Registry and bundle all three backends:

```sh
docker run --rm -p 127.0.0.1:6969:6969 \
  --mount type=volume,src=anisette-android-data,dst=/data \
  ghcr.io/fdkevin0/anisette-server-rs:latest --backend android
```

First startup provisions a device identity with Apple (`https://gsa.apple.com`) and persists it in `/data`. Keep the named volume across restarts to reuse the provisioned identity.

Verify the running server:

```sh
curl -s http://127.0.0.1:6969/
curl -s http://127.0.0.1:6969/v3/client_info
```

### Switching backends

`--backend <android|macos|windows>` sets the container's default backend (`android` by default). You can also override the backend per HTTP/WebSocket request using the `?backend=android|macos|windows` query parameter on a single running container:

```sh
# Use the container's default backend (android)
curl -s http://127.0.0.1:6969/

# Request macOS or Windows headers on the fly via query parameter
curl -s "http://127.0.0.1:6969/?backend=macos"
curl -s "http://127.0.0.1:6969/v3/client_info?backend=windows"
```

Or start a container with a different default `--backend`:

```sh
# macOS adid as default backend
docker run --rm -p 127.0.0.1:6969:6969 \
  --mount type=volume,src=anisette-macos-data,dst=/data \
  ghcr.io/fdkevin0/anisette-server-rs:latest --backend macos
```

All three backends support V1 (`GET /`) directly. Non-default V1 backends are provisioned lazily on first request and store their state under `/data/<backend>/identity.json`. Standard unmodified V3 clients (AltStore, SideStore) can use `android` or `windows`; provisioning new macOS devices over V3 requires the `X-Apple-I-MD-RINFO` finish-provisioning field (see [Compatibility](docs/compatibility.md)).

## API endpoints

All four endpoints accept an optional `?backend=android|macos|windows` query parameter (defaulting to the server's `--backend` startup flag):

| Endpoint | Method | Description |
| --- | --- | --- |
| `/` | `GET` | Returns Anisette V1 headers (`X-Apple-I-MD`, `X-Apple-I-MD-M`, `X-MMe-Client-Info`, etc.) using the server's persisted identity |
| `/v3/client_info` | `GET`, `HEAD` | Returns `client_info` and `user_agent` strings for V3 clients |
| `/v3/get_headers` | `POST` | Generates V3 OTP headers from client-supplied Base64 `identifier` and `adi_pb` |
| `/v3/provisioning_session` | `GET` (WebSocket) | Interactive V3 device provisioning session |

## Documentation

| Topic | Guide |
| --- | --- |
| Docker deployment, CLI flags, proxies, and state storage | [Running and deployment](docs/operations.md) |
| Backend comparison, V3 routing differences, and host support | [Compatibility](docs/compatibility.md) |
| V1/V3 wire format, validation rules, and resource budgets | [Protocol reference](docs/protocol.md) |
| Preparing Apple ADI binaries for non-Docker local builds | [Asset preparation](assets/README.md) |
| Building, offline test suite, and live backend verification | [Development and validation](docs/development.md) |
| Upstream references and license inventory | [Provenance](docs/upstream.md), [Third-party notices](THIRD_PARTY_NOTICES.md), [Dependency licenses](docs/licenses.md) |

## Acknowledgements

This project builds upon the protocol specifications, reverse-engineering research, and emulator models from the following projects:

- **[Dadoum/anisette-v3-server](https://github.com/Dadoum/anisette-v3-server)** — Reference Anisette V1 and V3 HTTP/WebSocket protocol contract.
- **[SideStore/apple-private-apis](https://github.com/SideStore/apple-private-apis)** — `omnisette` V3 client behavior and opaque `adi_pb` state persistence reference.
- **[malmeloo/Anisette.py](https://github.com/malmeloo/Anisette.py)** — Android ARM64 Unicorn VM layout, symbol hooks, and RemoteADI client reference.
- **[sacca97/os-findmy-e](https://github.com/sacca97/os-findmy-e)** — macOS `adid` Mach-O loader, CoreADI dispatcher framing, guest heap allocator, and CoreFoundation/XPC stub models.
