FROM debian:bookworm-slim

ARG MUSL_VERSION=1.2.4
ARG MUSL_SHA256=7a35eae33d5372a7c0da1188de798726f68825513b7ae3ebe97aaaa52114f039

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl e2fsprogs gcc gcc-riscv64-linux-gnu libc6-dev make \
 && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL -o /tmp/musl.tar.gz "https://musl.libc.org/releases/musl-${MUSL_VERSION}.tar.gz" \
 && echo "${MUSL_SHA256}  /tmp/musl.tar.gz" | sha256sum -c - \
 && tar -xzf /tmp/musl.tar.gz -C /tmp \
 && cd "/tmp/musl-${MUSL_VERSION}" \
 && CC=riscv64-linux-gnu-gcc CROSS_COMPILE=riscv64-linux-gnu- CFLAGS="-Os" ./configure --target=riscv64-linux-gnu --prefix=/opt/musl --disable-shared >/dev/null \
 && make -j"$(nproc)" install >/dev/null \
 && rm -rf /tmp/musl.tar.gz "/tmp/musl-${MUSL_VERSION}"

ARG RUST_VERSION=1.94.0
# RUSTUP_TOOLCHAIN to overrides rust-toolchain.toml, because we don't need the extra components here.
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH RUSTUP_TOOLCHAIN=${RUST_VERSION}
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y -q --profile minimal --no-modify-path \
      --default-toolchain "${RUST_VERSION}" --target riscv64gc-unknown-none-elf \
 && chmod -R a+rX /opt/rustup /opt/cargo

ARG CBINDGEN_VERSION=0.29.4
RUN cargo install -q --locked --version "${CBINDGEN_VERSION}" cbindgen \
 && rm -rf /opt/cargo/registry /opt/cargo/git \
 && chmod -R a+rX /opt/cargo

# For the build scripts in src/main/rust.
ENV CC_riscv64gc_unknown_none_elf=riscv64-linux-gnu-gcc AR_riscv64gc_unknown_none_elf=riscv64-linux-gnu-ar MUSL=/opt/musl
