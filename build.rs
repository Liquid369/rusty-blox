fn main() {
    cc::Build::new()
        .file("src/quark/quark.c") // or .cpp
        .include("src/quark") // include directory for headers
        .compile("quark");
}