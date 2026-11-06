FROM rust:1.99.1-bookworm AS chef
RUN apt-get update && apt-get install -y --no-install-recommends cmake clang libclang-dev ninja-build \
    && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef --version 0.1.78 --locked
WORKDIR /src

FROM chef AS planner
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS build
COPY --from=planner /src/recipe.json recipe.json
# Cook example dependencies too; cargo-chef has no named-example selector.
RUN cargo chef cook --locked --release --recipe-path recipe.json \
    -p anisette-server -p adi-android -p adi-windows --bin anisette-server --examples
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
RUN cargo build --locked --release \
    -p anisette-server -p adi-android -p adi-windows \
    --bin anisette-server --example import_apk --example import_itunes

FROM build AS assets
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl p7zip-full \
    && rm -rf /var/lib/apt/lists/*
RUN curl --disable --fail --location --proto '=https' --proto-redir '=https' \
      --connect-timeout 30 --max-time 180 --max-filesize 150000000 \
      --output /tmp/applemusic.apk \
      https://apps.mzstatic.com/content/android-apple-music-apk/applemusic.apk \
    && target/release/examples/import_apk /tmp/applemusic.apk /assets \
    && rm /tmp/applemusic.apk
RUN curl --disable --fail --location --proto '=https' --proto-redir '=https' \
      --connect-timeout 30 --max-time 180 --max-filesize 210000000 \
      --output /tmp/iTunes64Setup.exe \
      https://secure-appldnld.apple.com/itunes12/142-28289-20261026-6acff6c8-72eb-4045-84eb-fd35239ac645/iTunes64Setup.exe \
    && target/release/examples/import_itunes /tmp/iTunes64Setup.exe /assets/CoreADI.dll \
    && rm /tmp/iTunes64Setup.exe \
    && chmod 0755 /assets \
    && chmod 0644 /assets/libCoreADI.so /assets/libstoreservicescore.so /assets/CoreADI.dll

# The author's download page serves the binary through its empty POST form.
RUN curl --disable --fail --location --proto '=https' --proto-redir '=https' \
      --connect-timeout 30 --max-time 120 --max-filesize 3677696 --data '' \
      --output /assets/CoreADI-26.x86_64 https://sharebin.eu/files/CoreADI.x86_64 \
    && echo '4b6b320f08edcf5d65ed1141399b76a8d9323fee2045c577c5d2f30f35a5dae1  /assets/CoreADI-26.x86_64' \
      | sha256sum --check \
    && chmod 0644 /assets/CoreADI-26.x86_64

FROM scratch AS binaries
COPY --from=build /src/target/release/anisette-server /anisette-server
COPY --from=build /src/target/release/examples/import_apk /import_apk
COPY --from=build /src/target/release/examples/import_itunes /import_itunes

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --create-home anisette \
    && install -d -m 0700 -o anisette -g anisette /data
COPY --from=build /src/target/release/anisette-server /usr/local/bin/anisette-server
COPY --from=assets /assets /assets
ENV ANISETTE_LISTEN=0.0.0.0:6969 ANISETTE_STATE_DIR=/data
USER 10001:10001
RUN anisette-server --backend android --check \
    && anisette-server --backend macos --check \
    && anisette-server --backend windows --check
EXPOSE 6969
ENTRYPOINT ["anisette-server"]
CMD ["--backend", "android"]
