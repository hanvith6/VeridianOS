fn main() {
    // Use workspace-root-relative path for the linker script
    println!("cargo:rustc-link-arg=-Tuser_programs/hello/src/linker.ld");
    println!("cargo:rerun-if-changed=user_programs/hello/src/linker.ld");
}
