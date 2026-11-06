# V1/V3 protocol reference

The HTTP and WebSocket routes follow [Dadoum/anisette-v3-server (`source/app.d`)](https://github.com/Dadoum/anisette-v3-server/blob/684895737aaf1a1c4586fcd5b67509ffd500a36c/source/app.d) and [SideStore's `omnisette` V3 client](https://github.com/SideStore/apple-private-apis/blob/03beb1aa42991ccdad6214dee77e72282bef461f/omnisette/src/remote_anisette_v3.rs).

| Endpoint | Methods | Input | Output |
| --- | --- | --- | --- |
| `/` | `GET` | None | V1 headers JSON object |
| `/v3/client_info` | `GET`, `HEAD` | None | `{"client_info":"...","user_agent":"..."}` |
| `/v3/get_headers` | `POST` | `{"identifier":"<Base64>","adi_pb":"<Base64>"}` | `{"result":"Headers","X-Apple-I-MD":"...","X-Apple-I-MD-M":"...","X-Apple-I-MD-RINFO":"..."}` |
| `/v3/provisioning_session` | `GET` (WebSocket) | Interactive JSON handshake below | Interactive JSON handshake below |

## V3 provisioning handshake

```text
server -> {"result":"GiveIdentifier"}
client -> {"identifier":"<Base64 of 16 bytes>"}
server -> {"result":"GiveStartProvisioningData"}
client -> {"spim":"<Base64>"}
server -> {"result":"GiveEndProvisioningData","cpim":"<Base64>"}
client -> {"ptm":"<Base64>","tk":"<Base64>"}
server -> {"result":"ProvisioningSuccess","adi_pb":"<Base64>"}
server closes connection
```

## Identifiers and state envelopes

- `identifier` is standard padded Base64 encoding of 16 raw bytes. UUID formatting preserves byte order without Windows mixed-endian swapping. Android passes the first 16 characters of the uppercase hyphenated UUID string to ADI. The ADI `DSID` is `-2i64` (`0xfffffffffffffffe`).
- `adi_pb` is Base64-encoded state stored opaquely by the V3 client:
  - `android`: raw `adi.pb` file bytes.
  - `macos`: JSON envelope (`provider: "macos-adid"`, `version: 1`) bundling virtual filesystem files.
  - `windows`: JSON envelope (`provider: "windows-coreadi"`, `version: 1`) bundling virtual filesystem files.
- V1 (`GET /`) uses the server's own persisted identity (`identity.json`) and does not share state with V3 clients.

## V1 response fields

`GET /` returns a JSON object with:
`X-Apple-I-Client-Time`, `X-Apple-I-MD`, `X-Apple-I-MD-M`, `X-Apple-I-MD-RINFO`, `X-Apple-I-MD-LU`, `X-Apple-I-SRL-NO`, `X-MMe-Client-Info`, `X-Apple-I-TimeZone`, `X-Apple-Locale`, `X-Mme-Device-Id`, and `User-Agent`.

## Error responses

- `POST /v3/get_headers`: Input validation and backend errors return HTTP `200 OK` with `{"result":"GetHeadersError","message":"..."}` to match the reference V3 contract.
- `WS /v3/provisioning_session`: Sends `InvalidIdentifier`, `Timeout`, `StartProvisioningError`, or `EndProvisioningError` (with `message` on start/end errors) and closes the WebSocket.

## Validation and resource budgets

- `identifier` must decode to exactly 16 bytes using canonical padded standard Base64.
- Binary provisioning fields (`spim`, `ptm`, `tk`) are bounded to 64 KiB decoded, and `adi_pb` is bounded to 3 MiB decoded; HTTP request bodies and WebSocket messages are bounded to 4 MiB + 4 KiB (`413 Payload Too Large` on oversized HTTP bodies).
- WebSocket upgrades reject browser `Origin` headers (`400 Bad Request`) to prevent cross-site WebSocket hijacking on loopback.
- All HTTP responses include `Cache-Control: no-store` and `Implementation-Version: anisette-server-rs/<version>`.
- WebSocket stages wait at most 3 seconds for each client message and 3 seconds per send; control frames (`Ping`/`Pong`) are bounded to 16 per read and do not extend stage deadlines.
- Concurrency budgets: 64 TCP connections (10s header timeout), 32 in-flight HTTP requests, 4 V3 OTP workers, 4 concurrent V3 provisioning sessions, and 1 V1 worker (queue depth 4). Exhausted capacity returns `503 Service Unavailable`.
- Each native Unicorn execution is bounded to 5 seconds or 50 million instructions.

## `X-Apple-I-MD-RINFO` extension in V3 finish messages

In addition to `ptm` and `tk`, the client's final provisioning message may include `"X-Apple-I-MD-RINFO"` as a positive `u64` integer or decimal string forwarded from Apple's finish-provisioning response:

- **macOS and Windows**: Required because the pinned native binaries return `0` for routing after standard `ptm`/`tk` completion.
- **Android**: Optional; defaults to `17106176` when omitted, or stores Apple's value when provided.
