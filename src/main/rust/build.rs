// SPDX-License-Identifier: MIT
//! cbindgen and kprintf.c + TLSF compilation into libkernel_libc.a. We don't link libkernel_libc.a directly,
//! because linker command line argument order matters for our shadowed symbols. Also see openos2/build.rs.

#[path = "build_c.rs"]
mod build_c;

use std::fs;
use std::process::Command;

fn main() {
    println!("cargo::rerun-if-changed=kernel/api");
    println!("cargo::rerun-if-changed=cbindgen.toml");
    println!("cargo::rerun-if-changed=../c/kprintf.c");
    println!("cargo::rerun-if-changed=../../../external/tlsf");
    let Some(tc) = build_c::Toolchain::from_env("../../..") else { return };

    let include = tc.out.join("include");
    fs::create_dir_all(&include).unwrap();
    let status = Command::new("cbindgen")
        .args(["-q", "--config", "cbindgen.toml", "--output"])
        .arg(include.join("kernel.h"))
        .arg("kernel/api/mod.rs")
        .status()
        .unwrap();
    assert!(status.success());

    let include_flag = format!("-I{}", include.display());
    tc.archive(
        "kernel_libc",
        &[
            (tc.path("src/main/c/kprintf.c"), tc.cflags(&["-std=gnu11", "-Wall", "-Wextra", &include_flag])),
            (tc.path("external/tlsf/tlsf.c"), tc.cflags(&["-std=gnu11", "-DNDEBUG"])),
        ],
    );
    println!("cargo::metadata=include={}", include.display());
    println!("cargo::metadata=lib={}", tc.out.display());
}
