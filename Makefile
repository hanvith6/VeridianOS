# VeridianOS Makefile
# Builds the kernel, user programs, and disk image, then runs under QEMU.

.PHONY: all build disk run clean clippy fmt build_agent_test build_policy_test build_smp_test

DISK_IMG := disk.img
HELLO_ELF := target/riscv64gc-unknown-none-elf/release/hello

all: disk

# --- Build Targets ---

# Build the user_program (legacy inline ELF, kept for compatibility)
build_user_program:
	cargo build -p user_program --release

# Build the hello user-space process for the disk image
build_hello:
	cargo build -p hello --release

# Build the neural_test user-space process for the disk image
build_neural_test:
	cargo build -p neural_test --release

# Build the semantic_test user-space process for the disk image
build_semantic_test:
	cargo build -p semantic_test --release

# Build the agent_test user-space process for the disk image
build_agent_test:
	cargo build -p agent_test --release

# Build the policy_test user-space process for the disk image
build_policy_test:
	cargo build -p policy_test --release

# Build the smp_test user-space process for the disk image
build_smp_test:
	cargo build -p smp_test --release

# Build the kernel (depends on user_program for include_bytes! path)
build_kernel: build_user_program
	cargo build -p veridian-kernel --release

# Full build: everything including disk image
build: disk build_kernel

# --- Disk Image ---

# Create disk.img: a POSIX ustar TAR archive containing all user-space programs.
# The TAR format is understood by the kernel's InitRAMFS parser.
disk: build_hello build_neural_test build_semantic_test build_agent_test build_policy_test build_smp_test
	@echo "[DISK] Building disk image: $(DISK_IMG)"
	@# Remove stale image if it exists
	@rm -f $(DISK_IMG)
	@# Create a POSIX ustar TAR containing all user-space ELF binaries
	@# We use --format=ustar to ensure the kernel parser gets a known format
	cd target/riscv64gc-unknown-none-elf/release && tar cf ../../../$(DISK_IMG) --format=ustar hello neural_test semantic_test agent_test policy_test smp_test
	@echo "[DISK] Created $(DISK_IMG) (ustar TAR):"
	@tar tf $(DISK_IMG)
	@ls -lh $(DISK_IMG)


# --- Run ---

# Build everything and boot in QEMU
run: build
	cargo run -p veridian-kernel --release

# --- Debug ---

# Run with GDB server enabled (connect with: riscv64-unknown-elf-gdb)
debug: build
	qemu-system-riscv64 \
		-machine virt \
		-nographic \
		-serial mon:stdio \
		-bios default \
		-smp 4 \
		-device virtio-net-device \
		-netdev user,id=net0 \
		-drive id=hd0,file=$(DISK_IMG),format=raw,if=none \
		-device virtio-blk-device,drive=hd0 \
		-kernel target/riscv64gc-unknown-none-elf/release/veridian-kernel \
		-s -S

# --- Utility ---

clean:
	cargo clean
	rm -f $(DISK_IMG)

clippy:
	cargo clippy --workspace --target=riscv64gc-unknown-none-elf

fmt:
	cargo fmt --all --check
