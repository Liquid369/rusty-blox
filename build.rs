fn main() {
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