fn main() {
    println!("cargo:rustc-link-arg=-Tuser_programs/smp_test/src/linker.ld");
    println!("cargo:rerun-if-changed=user_programs/smp_test/src/linker.ld");
}
