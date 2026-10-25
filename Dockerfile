FROM debian:bookworm-slim

# musl provides the C library parts the kernel links: string, math, snprintf, strtod, setjmp.
ARG MUSL_VERSION=1.2.4
ARG MUSL_SHA256=7a35eae33d5372a7c0da1188de798726f68825513b7ae3ebe97aaaa52114f039

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      binutils-riscv64-linux-gnu ca-certificates cpp-12-riscv64-linux-gnu curl e2fsprogs \
      gcc-12-riscv64-linux-gnu libgcc-12-dev-riscv64-cross make \
 && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL -o /tmp/musl.tar.gz "https://musl.libc.org/releases/musl-${MUSL_VERSION}.tar.gz" \
 && echo "${MUSL_SHA256}  /tmp/musl.tar.gz" | sha256sum -c - \
 && tar -xzf /tmp/musl.tar.gz -C /tmp \
 && cd "/tmp/musl-${MUSL_VERSION}" \
 && CC=riscv64-linux-gnu-gcc-12 CROSS_COMPILE=riscv64-linux-gnu- CFLAGS="-Os" ./configure --target=riscv64-linux-gnu --prefix=/opt/musl --disable-shared >/dev/null \
 && make -j"$(nproc)" install >/dev/null \
 && rm -rf /tmp/musl.tar.gz "/tmp/musl-${MUSL_VERSION}"
