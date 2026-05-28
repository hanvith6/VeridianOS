fn main() {
    // Configure Cargo to link the kernel using its specific RISC-V linker script
    println!("cargo:rustc-link-arg=-Tkernel/src/arch/riscv64/linker.ld");
    // Rebuild if the linker script changes
    println!("cargo:rerun-if-changed=kernel/src/arch/riscv64/linker.ld");
}
