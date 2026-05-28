fn main() {
    // Configure Cargo to link the user program using its specific linker script
    println!("cargo:rustc-link-arg=-Tuser_program/src/linker.ld");
    // Rebuild if the linker script changes
    println!("cargo:rerun-if-changed=user_program/src/linker.ld");
}
