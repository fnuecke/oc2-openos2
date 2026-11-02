# OpenOS II

A small operating system for [OpenComputers II](https://github.com/fnuecke/oc2)'s RISC-V computers. The kernel runs Lua 5.4 directly on the virtual hardware, the userland is Lua scripts and a shell written in Lua. The kernel itself is agnostic and could theoretically also be used for other single-process applications running on top of it.

## Building

Requires Docker and a JDK 21.

```bash
./gradlew build
```

Gradle builds the toolchain container from the `Dockerfile` and runs `cargo xtask` in it, which writes the kernel `Image` and the disk image `disk.img` to `build/os`.

Clone with `--recursive`, or run `git submodule update --init` after cloning. Lua, lua-cjson and TLSF are submodules in `external/`.

The kernel needs a firmware that loads it at the `text_offset` in its Image header, 64 KiB above RAM base. This means Sedna's firmware from `sedna-buildroot` (`boot/sedna-fw`) can also load the OpenOS II kernel.
