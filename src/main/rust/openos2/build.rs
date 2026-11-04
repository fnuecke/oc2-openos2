// SPDX-License-Identifier: MIT

#[path = "../build_c.rs"]
mod build_c;

use std::env;

const LUA: &[&str] = &[
    "lapi", "lcode", "lctype", "ldebug", "ldo", "ldump", "lfunc", "lgc", "llex", "lmem", "lobject", "lopcodes",
    "lparser", "lstate", "lstring", "ltable", "ltm", "lundump", "lvm", "lzio", "lauxlib", "lbaselib", "lcorolib",
    "ldblib", "lmathlib", "loadlib", "lstrlib", "ltablib", "lutf8lib", "liolib", "loslib",
];
/// Speed vs. size tradeoff: only ones with -O2, rest is -Os.
const LUA_O2: &[&str] = &["lapi", "ldo", "lstrlib", "ltable"];

fn main() {
    println!("cargo::rerun-if-changed=../../c/lua");
    println!("cargo::rerun-if-changed=../../c/kernel.ld");
    println!("cargo::rerun-if-changed=../../../../external/lua");
    println!("cargo::rerun-if-changed=../../../../external/lua-cjson");
    let Some(tc) = build_c::Toolchain::from_env("../../../..") else { return };

    let include = format!("-I{}", env::var("DEP_KERNEL_INCLUDE").unwrap());
    let lua = format!("-I{}", tc.path("external/lua").display());
    let lua_conf = tc.path("src/main/c/lua/lua_conf.h").display().to_string();
    let app = tc.cflags(&["-std=gnu11", "-Wall", "-Wextra", "-flto", &include, &lua, "-include", &lua_conf]);
    let lua_flags = tc.cflags(&["-std=gnu99", "-Wall", "-flto", &include, "-include", &lua_conf]);
    let cjson = tc.cflags(&["-std=gnu99", "-Wall", "-flto", &include, "-include", &lua_conf, &lua]);

    let mut units = Vec::new();
    for name in ["app", "luaapi", "lnative", "lposix", "lsignal"] {
        units.push((tc.path(&format!("src/main/c/lua/{name}.c")), app.clone()));
    }
    for name in LUA {
        let mut flags = lua_flags.clone();
        if LUA_O2.contains(name) {
            flags.push("-O2".to_string());
        }
        units.push((tc.path(&format!("external/lua/{name}.c")), flags));
    }
    for name in ["lua_cjson", "strbuf", "fpconv"] {
        units.push((tc.path(&format!("external/lua-cjson/{name}.c")), cjson.clone()));
    }
    tc.archive("app", &units);

    println!("cargo::rustc-link-search=native={}", tc.out.display());
    println!("cargo::rustc-link-search=native={}", env::var("DEP_KERNEL_LIB").unwrap());
    println!("cargo::rustc-link-search=native={}/lib", tc.musl.display());
    for lib in ["app", "kernel_libc", "c", "gcc"] {
        println!("cargo::rustc-link-lib=static={lib}");
    }
    for arg in [
        "-march=rv64imafdc_zicsr_zifencei",
        "-mabi=lp64d",
        "-static",
        "-nostdlib",
        "-no-pie",
        "-Wl,--gc-sections",
        "-flto",
        "-Wl,--build-id=none",
        &format!("-Wl,-Map,{}/kernel.map", tc.out.display()),
        &format!("-T{}", tc.path("src/main/c/kernel.ld").display()),
    ] {
        println!("cargo::rustc-link-arg-bins={arg}");
    }
}
