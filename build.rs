fn main() {
    // `enable -d` dlcloses the library, but readline keeps pointers to the
    // commands inkline registered. Keep the library mapped so they stay valid.
    println!("cargo:rustc-cdylib-link-arg=-Wl,-z,nodelete");
}
