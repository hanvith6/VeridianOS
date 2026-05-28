fn main() {
    // Use workspace-root-relative path for the linker script
    println!("cargo:rustc-link-arg=-Tuser_programs/semantic_test/src/linker.ld");
    println!("cargo:rerun-if-changed=user_programs/semantic_test/src/linker.ld");
}
