use std::process::Command;

fn emit_env(var: &str, cmd: &str, args: &[&str]) {
    if let Ok(out) = Command::new(cmd).args(args).output() {
        if out.status.success() {
            let val = String::from_utf8_lossy(&out.stdout);
            println!("cargo:rustc-env={}={}", var, val.trim());
        }
    }
}

fn main() {
    emit_env(
        "RUSTYBLOX_GIT_COMMIT",
        "git",
        &["rev-parse", "--short", "HEAD"],
    );
    emit_env(
        "RUSTYBLOX_BUILD_TIME",
        "date",
        &["-u", "+%Y-%m-%dT%H:%M:%SZ"],
    );
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads");

    cc::Build::new()
        .file("src/quark/quark.c") // or .cpp
        .file("src/quark/blake.c")
        .file("src/quark/bmw.c")
        .file("src/quark/groestl.c")
        .file("src/quark/jh.c")
        .file("src/quark/keccak.c")
        .file("src/quark/skein.c")
        .include("src/quark") // include directory for headers
        .compile("libquark.a");
}
