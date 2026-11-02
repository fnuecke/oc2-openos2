// SPDX-License-Identifier: MIT

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const OBJCOPY: &str = "riscv64-linux-gnu-objcopy";
const OBJDUMP: &str = "riscv64-linux-gnu-objdump";
const AR: &str = "riscv64-linux-gnu-ar";

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
    let build = root.join("build/os");
    let target = root.join("build/cargo");
    let artifacts = cargo_build(&root, &target);
    let kernel = build.join("kernel");
    fs::create_dir_all(kernel.join("lib")).unwrap();
    fs::create_dir_all(kernel.join("include")).unwrap();

    let elf = build.join("kernel.elf");
    fs::copy(&artifacts.executable, &elf).unwrap();
    let map = fs::read_to_string(artifacts.openos2_out.join("kernel.map")).unwrap();
    fs::write(build.join("kernel.map"), &map).unwrap();

    // The C library functions come from musl or the kernel, see openos2/build.rs.
    assert!(!map.contains(".rlib(compiler_builtins"), "nothing may come from compiler_builtins");

    // Nothing in libc.a may make a system call or read the thread pointer: an ecall goes to the firmware.
    let disassembly = output(Command::new(OBJDUMP).arg("-d").arg(&elf));
    let count = |word: &str| {
        disassembly.lines().filter(|l| l.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == word)).count()
    };

    assert_eq!(count("ecall"), 1, "ecall: only sbi_set_timer");
    assert_eq!(count("tp"), 2, "tp: only the trap frame");

    run(Command::new(OBJCOPY).args(["-O", "binary"]).arg(&elf).arg(build.join("Image")));
    run(Command::new(root.join("mkdisk.sh"))
        .env("SIZE", "2M")
        .arg(build.join("disk.img"))
        .arg(build.join("Image"))
        .arg(root.join("src/main/rootfs")));

    let members = output(Command::new(AR).arg("t").arg(&artifacts.staticlib));
    let member: Vec<_> = members.lines().filter(|m| m.starts_with("kernel_c-")).collect();
    assert_eq!(member.len(), 1, "one LTO object in {}", artifacts.staticlib.display());
    let object = target.join("kernel.o");
    fs::write(&object, bytes(Command::new(AR).arg("p").arg(&artifacts.staticlib).arg(member[0]))).unwrap();

    let library = kernel.join("lib/libkernel.a");
    let _ = fs::remove_file(&library);
    let c_objects = artifacts.kernel_out.join("kernel_libc");
    run(Command::new(AR)
        .arg("rcs")
        .arg(&library)
        .arg(&object)
        .arg(c_objects.join("kprintf.o"))
        .arg(c_objects.join("tlsf.o")));

    fs::copy(root.join("src/main/c/kernel.ld"), kernel.join("lib/kernel.ld")).unwrap();
    fs::copy(artifacts.kernel_out.join("include/kernel.h"), kernel.join("include/kernel.h")).unwrap();
    fs::copy(root.join("src/main/c/kernel.mk"), kernel.join("kernel.mk")).unwrap();

    run(Command::new("make").arg("-s").arg("-C").arg(root.join("examples")));
}

struct Artifacts {
    executable: PathBuf,
    staticlib: PathBuf,
    kernel_out: PathBuf,
    openos2_out: PathBuf,
}

fn cargo_build(root: &Path, target: &Path) -> Artifacts {
    let json = output(
        Command::new(env!("CARGO"))
            .current_dir(root.join("src/main/rust"))
            .args(["build", "--release", "--workspace", "--message-format=json-render-diagnostics", "--target-dir"])
            .arg(target),
    );
    let (mut executable, mut staticlib, mut kernel_out, mut openos2_out) = (None, None, None, None);
    for line in json.lines() {
        let package = string(line, "package_id").map(package_name);
        if line.contains(r#""reason":"build-script-executed""#) {
            let out_dir = string(line, "out_dir").map(PathBuf::from);
            match package {
                Some("kernel") => kernel_out = out_dir,
                Some("openos2") => openos2_out = out_dir,
                _ => {}
            }
        } else if line.contains(r#""reason":"compiler-artifact""#) {
            match package {
                Some("openos2") if line.contains(r#""kind":["bin"]"#) => {
                    executable = string(line, "executable").map(PathBuf::from)
                }
                Some("kernel-c") => staticlib = string(line, "filenames").map(PathBuf::from),
                _ => {}
            }
        }
    }
    Artifacts {
        executable: executable.unwrap(),
        staticlib: staticlib.unwrap(),
        kernel_out: kernel_out.unwrap(),
        openos2_out: openos2_out.unwrap(),
    }
}

fn package_name(id: &str) -> &str {
    let (path, fragment) = id.split_once('#').unwrap();
    match fragment.split_once('@') {
        Some((name, _)) => name,
        None => path.rsplit('/').next().unwrap(),
    }
}

fn string<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = &line[line.find(&format!(r#""{key}":"#))? + key.len() + 3..];
    let rest = rest.strip_prefix('[').unwrap_or(rest).strip_prefix('"')?;
    Some(&rest[..rest.find('"')?])
}

fn run(command: &mut Command) {
    assert!(command.status().unwrap().success(), "{command:?} failed");
}

fn output(command: &mut Command) -> String {
    String::from_utf8(bytes(command)).unwrap()
}

fn bytes(command: &mut Command) -> Vec<u8> {
    let output = command.stderr(Stdio::inherit()).output().unwrap();
    assert!(output.status.success(), "{command:?} failed");
    output.stdout
}
