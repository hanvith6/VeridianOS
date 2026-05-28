fn main() {
    // Use workspace-root-relative path for the linker script
    println!("cargo:rustc-link-arg=-Tuser_programs/neural_test/src/linker.ld");
    println!("cargo:rerun-if-changed=user_programs/neural_test/src/linker.ld");
}
