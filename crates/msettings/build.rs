fn main() {
    // Paks link this as DT_NEEDED "libmsettings.so"; the SONAME must
    // match or the dynamic linker won't resolve it at their exec time.
    println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libmsettings.so");
}
