// SPDX-License-Identifier: MIT

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub struct Toolchain {
    cc: String,
    ar: String,
    gcc_include: String,
    pub musl: PathBuf,
    pub root: PathBuf,
    pub out: PathBuf,
}

impl Toolchain {
    /// `root` is the repository root, relative to the package.
    pub fn from_env(root: &str) -> Option<Self> {
        println!("cargo::rerun-if-env-changed=CC_riscv64gc_unknown_none_elf");
        println!("cargo::rerun-if-env-changed=AR_riscv64gc_unknown_none_elf");
        println!("cargo::rerun-if-env-changed=MUSL");
        let cc = env::var("CC_riscv64gc_unknown_none_elf").ok()?;
        let root = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join(root).canonicalize().unwrap();
        // Written by Gradle when the toolchain container changes.
        println!("cargo::rerun-if-changed={}", root.join("build/toolchain-image.id").display());
        let output = Command::new(&cc).arg("-print-file-name=include").output().unwrap();
        let gcc_include = String::from_utf8(output.stdout).unwrap().trim().to_string();
        Some(Self {
            cc,
            gcc_include,
            ar: env::var("AR_riscv64gc_unknown_none_elf").unwrap(),
            musl: env::var("MUSL").unwrap().into(),
            root,
            out: env::var("OUT_DIR").unwrap().into(),
        })
    }

    pub fn path(&self, path: &str) -> PathBuf {
        self.root.join(path)
    }

    pub fn cflags(&self, extra: &[&str]) -> Vec<String> {
        let flags = "-march=rv64imafdc_zicsr_zifencei \
                     -mabi=lp64d \
                     -mcmodel=medlow \
                     -nostdinc \
                     -Os \
                     -g \
                     -fno-pie \
                     -fno-stack-protector \
                     -fno-asynchronous-unwind-tables \
                     -ffunction-sections \
                     -fdata-sections";
        let paths = [
            "-isystem".to_string(),
            format!("{}/include", self.musl.display()),
            "-isystem".to_string(),
            self.gcc_include.clone(),
            format!("-ffile-prefix-map={}=.", self.root.display()),
        ];
        flags.split_whitespace().map(String::from).chain(paths).chain(extra.iter().map(|s| s.to_string())).collect()
    }

    pub fn archive(&self, name: &str, units: &[(PathBuf, Vec<String>)]) {
        let dir = self.out.join(name);
        fs::create_dir_all(&dir).unwrap();

        let children: Vec<_> = units
            .iter()
            .map(|(source, flags)| {
                let object = dir.join(source.file_name().unwrap()).with_extension("o");
                let child = Command::new(&self.cc)
                    .args(flags)
                    .arg("-c")
                    .arg("-o")
                    .arg(&object)
                    .arg(source)
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                (object, child)
            })
            .collect();

        let mut objects = Vec::new();
        for (object, child) in children {
            let output = child.wait_with_output().unwrap();
            for line in String::from_utf8_lossy(&output.stderr).lines() {
                println!("cargo::warning={line}");
            }
            assert!(output.status.success(), "compiling {} failed", object.display());
            objects.push(object);
        }

        let archive = self.out.join(format!("lib{name}.a"));
        let _ = fs::remove_file(&archive);
        assert!(Command::new(&self.ar).arg("rcs").arg(&archive).args(&objects).status().unwrap().success());
    }
}
