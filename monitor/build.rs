fn main() {
    // Pass the custom linker script to the linker
    println!("cargo:rustc-link-arg=-Tmonitor/src/linker.ld");
    println!("cargo:rerun-if-changed=monitor/src/linker.ld");
}
