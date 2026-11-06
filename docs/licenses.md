# Dependency licenses

The repository retains GPL-2.0-only, including adapted os-findmy-e code. The production server uses Hyper/Tokio/Tungstenite for incoming requests and reqwest with rustls for Apple HTTPS provisioning. Reqwest, its TLS dependencies (including AWS-LC), and `sync_wrapper` are runtime dependencies; they are not limited to the integration-test client.

The normal dependency graph now includes Apache-2.0-only `sync_wrapper`. The earlier curl-based dependency separation no longer applies. This inventory records declared licenses, not a compatibility clearance for distributing the combined GPL-2.0-only binary; the Apache-2.0 compatibility issue must be resolved before distribution. See the [GNU license list](https://www.gnu.org/licenses/license-list.html#apache2). `clang-sys` remains a build dependency and is not included among normal runtime edges below.

The table was generated from `cargo metadata --locked --filter-platform TARGET --format-version 1`, traversing normal dependency edges from `anisette-server`, for x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu, aarch64-apple-darwin, and x86_64-pc-windows-msvc. Build/dev edges are excluded; proc macros reached through normal edges are included. Feature unification can include test-only features, so this is a conservative inventory rather than a linker map. Individual copyright/license files in dependency source packages remain authoritative.

Container base packages retain their own package licenses and are outside this Cargo table. Curl is used only by the image build to download assets. Apple binaries remain proprietary; bundling them grants no redistribution rights. Source provenance and retained notices are recorded in [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md).

Cargo.lock SHA-256: `0ba4c8fb5c0a23b12c1dda362b136a8f61a8c90b2b4c405db028667b575091f1`.

| Package | Version | Declared SPDX | Normal dependency targets |
| --- | --- | --- | --- |
| adi-android | 0.1.2 | GPL-2.0-only | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| adi-macos | 0.1.2 | GPL-2.0-only | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| adi-windows | 0.1.2 | GPL-2.0-only | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| anisette | 0.1.2 | GPL-2.0-only | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| anisette-server | 0.1.2 | GPL-2.0-only | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| anstream | 1.0.0 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| anstyle | 1.0.14 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| anstyle-parse | 1.0.0 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| anstyle-query | 1.1.5 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| anstyle-wincon | 3.0.11 | MIT OR Apache-2.0 | Windows x64 |
| atomic-waker | 1.1.2 | Apache-2.0 OR MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| aws-lc-rs | 1.19.0 | ISC AND (Apache-2.0 OR ISC) | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| aws-lc-sys | 0.47.0 | ISC AND (Apache-2.0 OR ISC) AND Apache-2.0 AND MIT AND BSD-3-Clause AND (Apache-2.0 OR ISC OR MIT) AND (Apache-2.0 OR ISC OR MIT-0) | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| base64 | 0.23.1 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| bitflags | 2.13.2 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| block-buffer | 0.12.1 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| bytes | 1.12.1 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| cfg-if | 1.0.5 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| chacha20 | 0.10.2 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| clap | 4.6.7 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| clap_builder | 4.6.7 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| clap_derive | 4.6.7 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| clap_lex | 1.1.1 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| colorchoice | 1.0.5 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| const-oid | 0.10.2 | Apache-2.0 OR MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| core-foundation | 0.10.1 | MIT OR Apache-2.0 | macOS ARM64 |
| core-foundation-sys | 0.8.7 | MIT OR Apache-2.0 | macOS ARM64 |
| cpufeatures | 0.3.1 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| crypto-common | 0.2.2 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| data-encoding | 2.11.1 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| deranged | 0.5.9 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| digest | 0.11.3 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| displaydoc | 0.2.7 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| equivalent | 1.0.3 | Apache-2.0 OR MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| errno | 0.3.14 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64 |
| fastrand | 2.5.0 | Apache-2.0 OR MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| form_urlencoded | 1.2.2 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| futures-channel | 0.3.34 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| futures-core | 0.3.34 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| futures-io | 0.3.34 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| futures-macro | 0.3.34 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| futures-sink | 0.3.34 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| futures-task | 0.3.34 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| futures-util | 0.3.34 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| getrandom | 0.2.17 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| getrandom | 0.4.3 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| goblin | 0.10.7 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| hashbrown | 0.17.1 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| heck | 0.5.0 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| http | 1.5.0 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| http-body | 1.1.0 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| http-body-util | 0.1.5 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| httparse | 1.10.1 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| httpdate | 1.0.3 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| hybrid-array | 0.4.15 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| hyper | 1.12.0 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| hyper-rustls | 0.27.10 | Apache-2.0 OR ISC OR MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| hyper-util | 0.1.21 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| icu_collections | 2.4.0 | Unicode-3.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| icu_locale_core | 2.4.0 | Unicode-3.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| icu_normalizer | 2.4.0 | Unicode-3.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| icu_normalizer_data | 2.4.0 | Unicode-3.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| icu_properties | 2.4.0 | Unicode-3.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| icu_properties_data | 2.4.0 | Unicode-3.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| icu_provider | 2.4.0 | Unicode-3.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| idna | 1.1.0 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| idna_adapter | 1.2.2 | Apache-2.0 OR MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| indexmap | 2.14.2 | Apache-2.0 OR MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| ipnet | 2.12.2 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| is_terminal_polyfill | 1.70.2 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| itoa | 1.0.18 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| libc | 0.2.191 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| linux-raw-sys | 0.12.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | Linux AMD64, Linux ARM64 |
| litemap | 0.8.3 | Unicode-3.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| log | 0.4.35 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| lru-slab | 0.1.3 | MIT OR Apache-2.0 OR Zlib | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| memchr | 2.8.3 | Unlicense OR MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| mio | 1.2.4 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| num-conv | 0.2.2 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| once_cell | 1.21.4 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| once_cell_polyfill | 1.70.2 | MIT OR Apache-2.0 | Windows x64 |
| openssl-probe | 0.2.2 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64 |
| percent-encoding | 2.3.2 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| pin-project-lite | 0.2.17 | Apache-2.0 OR MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| plain | 0.2.3 | MIT/Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| plist | 1.10.3 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| potential_utf | 0.1.6 | Unicode-3.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| powerfmt | 0.2.1 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| quick-xml | 0.42.0 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| quinn | 0.11.12 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| quinn-proto | 0.11.19 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| quinn-udp | 0.5.16 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| quote | 1.0.47 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| rand | 0.10.3 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| rand_core | 0.10.1 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| rand_pcg | 0.10.2 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| reqwest | 0.13.5 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| ring | 0.17.14 | Apache-2.0 AND ISC | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| rustc-hash | 2.1.3 | Apache-2.0 OR MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| rustix | 1.1.6 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | Linux AMD64, Linux ARM64, macOS ARM64 |
| rustls | 0.23.46 | Apache-2.0 OR ISC OR MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| rustls-native-certs | 0.8.5 | Apache-2.0 OR ISC OR MIT | Linux AMD64, Linux ARM64 |
| rustls-pki-types | 1.15.1 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| rustls-platform-verifier | 0.7.1 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| rustls-webpki | 0.103.16 | ISC | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| scroll | 0.13.0 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| scroll_derive | 0.13.2 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| security-framework | 3.7.0 | MIT OR Apache-2.0 | macOS ARM64 |
| security-framework-sys | 2.17.0 | MIT OR Apache-2.0 | macOS ARM64 |
| serde | 1.0.229 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| serde_core | 1.0.229 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| serde_derive | 1.0.229 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| serde_json | 1.0.154 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| sha1 | 0.11.0 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| sha2 | 0.11.1 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| signal-hook-registry | 1.4.8 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64 |
| slab | 0.4.12 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| smallvec | 1.16.4 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| socket2 | 0.6.5 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| stable_deref_trait | 1.2.1 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| strsim | 0.11.1 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| subtle | 2.6.1 | BSD-3-Clause | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| syn | 3.0.9 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| sync_wrapper | 1.0.2 | Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| synstructure | 0.14.0 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| tempfile | 3.27.0 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| thiserror-impl | 2.0.21 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| time | 0.3.55 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| time-core | 0.1.9 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| time-macros | 0.2.32 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| tinystr | 0.8.5 | Unicode-3.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| tinyvec | 1.13.3 | Zlib OR Apache-2.0 OR MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| tokio | 1.54.1 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| tokio-macros | 2.7.2 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| tokio-rustls | 0.26.6 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| tokio-tungstenite | 0.30.0 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| tower | 0.5.3 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| tower-http | 0.6.11 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| tower-layer | 0.3.3 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| tower-service | 0.3.3 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| tracing | 0.1.44 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| tracing-core | 0.1.36 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| try-lock | 0.2.5 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| tungstenite | 0.30.0 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| typenum | 1.20.1 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| unicode-ident | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| unicorn-engine | 2.1.5 | GPL-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| unicorn-engine-sys | 2.1.5 | GPL-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| untrusted | 0.9.0 | ISC | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| url | 2.5.8 | MIT OR Apache-2.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| utf8_iter | 1.1.0 | Apache-2.0 OR MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| utf8parse | 0.2.2 | Apache-2.0 OR MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| utf_types | 1.0.0 | Apache-2.0 OR MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| want | 0.3.2 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| windows-link | 0.2.1 | MIT OR Apache-2.0 | Windows x64 |
| windows-sys | 0.61.2 | MIT OR Apache-2.0 | Windows x64 |
| writeable | 0.6.4 | Unicode-3.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| yoke | 0.8.4 | Unicode-3.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| yoke-derive | 0.8.4 | Unicode-3.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| zerofrom | 0.1.8 | Unicode-3.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| zerofrom-derive | 0.1.8 | Unicode-3.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| zeroize | 1.9.1 | Apache-2.0 OR MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| zerotrie | 0.2.5 | Unicode-3.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| zerovec | 0.11.9 | Unicode-3.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| zerovec-derive | 0.11.7 | Unicode-3.0 | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
| zmij | 1.0.23 | MIT | Linux AMD64, Linux ARM64, macOS ARM64, Windows x64 |
