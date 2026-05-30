//! Physical Page Allocator for VeridianOS
//!
//! This module implements a robust Binary Buddy Allocator.
//! It manages RAM in page blocks ranging from order 0 (4KB) to order 10 (4MB).
//!
//! References:
//! - RISC-V Sv39 Virtual Memory Spec (§4.3)
//! - Binary Buddy Allocator design patterns

use spin::Mutex;

/// The size of a single physical page in bytes (4KB).
pub const PAGE_SIZE: usize = 4096;

/// A node in our free lists.
/// This structure is stored directly at the beginning of each free physical block.
#[repr(C)]
struct PageNode {
    next: Option<*mut PageNode>,
}

/// The state of the physical page allocator.
pub struct PageAllocatorState {
    free_lists: [Option<*mut PageNode>; 11],
    start_addr: usize,
    end_addr: usize,
}

// Manually implement Send and Sync because PageAllocatorState contains raw pointers (*mut PageNode)
unsafe impl Send for PageAllocatorState {}
unsafe impl Sync for PageAllocatorState {}

impl PageAllocatorState {
    const fn new() -> Self {
        Self {
            free_lists: [None; 11],
            start_addr: 0,
            end_addr: 0,
        }
    }

    /// Initialize the page allocator with a physical memory range.
    ///
    /// Parameters:
    /// - `start_addr`: The first address of RAM we can allocate.
    /// - `end_addr`: The upper boundary of RAM.
    fn init(&mut self, start_addr: usize, end_addr: usize) {
        // Align the start address UP to 4KB boundary
        let start_aligned = (start_addr + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        // Align the end address DOWN to 4KB boundary
        let end_aligned = end_addr & !(PAGE_SIZE - 1);

        self.start_addr = start_aligned;
        self.end_addr = end_aligned;

        // Reset the free lists
        self.free_lists = [None; 11];

        let mut curr_addr = start_aligned;
        while curr_addr < end_aligned {
            let mut selected_order = None;
            for order in (0..=10).rev() {
                let block_size = (1 << order) * PAGE_SIZE;
                if curr_addr.is_multiple_of(block_size) && curr_addr + block_size <= end_aligned {
                    selected_order = Some(order);
                    break;
                }
            }

            if let Some(order) = selected_order {
                let block_size = (1 << order) * PAGE_SIZE;
                unsafe {
                    self.push_front(order, curr_addr);
                }
                curr_addr += block_size;
            } else {
                break;
            }
        }
    }

    /// Helper to push a free block to the front of a specific order free list.
    unsafe fn push_front(&mut self, order: usize, addr: usize) {
        unsafe {
            let node_ptr = addr as *mut PageNode;
            let old_head = self.free_lists[order];
            core::ptr::write(node_ptr, PageNode { next: old_head });
            self.free_lists[order] = Some(node_ptr);
        }
    }

    /// Helper to pop the front block from a specific order free list.
    unsafe fn pop_front(&mut self, order: usize) -> Option<usize> {
        unsafe {
            let head_ptr = self.free_lists[order]?;
            let next_node = (*head_ptr).next;
            self.free_lists[order] = next_node;
            Some(head_ptr as usize)
        }
    }

    /// Helper to search and remove a specific block by its physical address from a free list.
    unsafe fn remove_block(&mut self, order: usize, target_addr: usize) -> bool {
        unsafe {
            let mut current = self.free_lists[order];
            let mut prev: Option<*mut PageNode> = None;

            while let Some(node_ptr) = current {
                if node_ptr as usize == target_addr {
                    let next_node = (*node_ptr).next;
                    if let Some(prev_ptr) = prev {
                        (*prev_ptr).next = next_node;
                    } else {
                        self.free_lists[order] = next_node;
                    }
                    (*node_ptr).next = None;
                    return true;
                }
                prev = Some(node_ptr);
                current = (*node_ptr).next;
            }
            false
        }
    }

    /// Allocate a block of pages of the given order.
    pub fn alloc_pages(&mut self, order: usize) -> Option<usize> {
        if order >= 11 {
            return None;
        }

        // 1. Try to get a block directly from the free list of the requested order
        unsafe {
            if let Some(addr) = self.pop_front(order) {
                // Zero the memory block before returning it to avoid leaking old data
                core::ptr::write_bytes(addr as *mut u8, 0, (1 << order) * PAGE_SIZE);
                return Some(addr);
            }
        }

        // 2. Look for a block in a higher order and split it
        for higher_order in (order + 1)..11 {
            unsafe {
                if let Some(block_addr) = self.pop_front(higher_order) {
                    let mut curr_order = higher_order;
                    let addr = block_addr;

                    while curr_order > order {
                        curr_order -= 1;
                        let block_size = (1 << curr_order) * PAGE_SIZE;
                        let buddy_addr = addr + block_size;
                        self.push_front(curr_order, buddy_addr);
                    }

                    // Zero the memory block before returning it to avoid leaking old data
                    core::ptr::write_bytes(addr as *mut u8, 0, (1 << order) * PAGE_SIZE);
                    return Some(addr);
                }
            }
        }

        None
    }

    /// Free a block of pages of the given order, performing recursive buddy merging.
    pub fn free_pages(&mut self, mut addr: usize, mut order: usize) {
        assert!(
            addr.is_multiple_of(PAGE_SIZE),
            "Address to free must be page-aligned"
        );
        assert!(
            addr >= self.start_addr && addr < self.end_addr,
            "Address 0x{:x} out of bounds [0x{:x}, 0x{:x})",
            addr,
            self.start_addr,
            self.end_addr
        );

        unsafe {
            while order < 10 {
                let block_size = (1 << order) * PAGE_SIZE;

                if !addr.is_multiple_of(block_size) {
                    panic!(
                        "Address 0x{:x} is not aligned to order {} block size (0x{:x})",
                        addr, order, block_size
                    );
                }

                let buddy_addr = addr ^ block_size;

                // Try to find and remove the buddy from the free list of the current order
                if self.remove_block(order, buddy_addr) {
                    // Buddy is free! Merge them into a single block of order + 1 at the lower address
                    addr = core::cmp::min(addr, buddy_addr);
                    order += 1;
                } else {
                    // Buddy is not free, stop merging
                    break;
                }
            }

            // Insert the merged block into the free list of the final order
            self.push_front(order, addr);
        }
    }
}

/// Globally accessible physical page allocator, protected by a spinlock.
static ALLOCATOR: Mutex<PageAllocatorState> = Mutex::new(PageAllocatorState::new());

/// Initialize the physical page allocator.
pub fn init(start_addr: usize, end_addr: usize) {
    ALLOCATOR.lock().init(start_addr, end_addr);
}

/// Allocate a block of physical pages of a specific order.
pub fn alloc_pages(order: usize) -> Option<usize> {
    ALLOCATOR.lock().alloc_pages(order)
}

/// Free a previously allocated block of physical pages of a specific order.
///
/// # Safety
/// The address must be a valid, allocated block of the specified order and must not be accessed again.
pub unsafe fn free_pages(addr: usize, order: usize) {
    ALLOCATOR.lock().free_pages(addr, order);
}

/// Allocate a 4KB physical page.
/// Returns the physical address, or `None` if out of memory.
pub fn alloc_page() -> Option<usize> {
    alloc_pages(0)
}

/// Free a previously allocated physical page.
///
/// # Safety
/// The address must be a valid, allocated 4KB page and must not be accessed again.
pub unsafe fn free_page(addr: usize) {
    unsafe {
        free_pages(addr, 0);
    }
}

/// Unit test verifying allocations, splits, merges, and alignments of the Buddy Allocator.
///
/// # Stack-allocation note
///
/// This function previously used `static mut TEST_BUF` to obtain a region whose address is
/// stable across the call.  That pattern requires `unsafe` and is fragile in a multi-boot
/// scenario.  It has been replaced with a stack-allocated array wrapped in
/// `core::mem::MaybeUninit`.  64 KB on the kernel stack is safe here because the kernel
/// boot path is called from a 512 KB stack (see `boot.S`), and this function runs before any
/// user code.  In the `#[cfg(test)]` module below the same region is heap-allocated via
/// `Box` so the host test runner never overflows its default 8 MB thread stack.
pub fn test_page_alloc() {
    #[cfg(not(test))]
    // boot-time test start

    // 64 KB, aligned to 64 KB so the allocator sees a single order-4 block.
    #[repr(C, align(65536))]
    struct TestBuffer([u8; 64 * 1024]);

    // Stack-allocate inside MaybeUninit — no UB from uninitialised bytes because
    // the allocator treats this memory as raw bytes and writes PageNode headers
    // into it via ptr::write before ever reading them.
    let mut buf = core::mem::MaybeUninit::<TestBuffer>::uninit();
    let start = buf.as_mut_ptr() as usize;
    let end = start + 64 * 1024;

    let mut allocator = PageAllocatorState::new();
    allocator.init(start, end);

    // Initial state check: since start is aligned to 64KB, there should be exactly one block of order 4
    for i in 0..4 {
        assert!(
            allocator.free_lists[i].is_none(),
            "Free list {} should be empty",
            i
        );
    }
    assert!(
        allocator.free_lists[4].is_some(),
        "Free list 4 should have the 64KB block"
    );

    // 1. Test basic allocation (order 0)
    let addr1 = allocator
        .alloc_pages(0)
        .expect("Should allocate order 0 page");
    assert!(
        addr1 >= start && addr1 < end,
        "Allocated address out of range"
    );
    assert_eq!(addr1, start, "First allocation should be at start address");

    // After splitting order 4 down to order 0, free lists should contain:
    // order 0: start + 4KB
    // order 1: start + 8KB
    // order 2: start + 16KB
    // order 3: start + 32KB
    assert!(
        allocator.free_lists[0].is_some(),
        "Order 0 should have the split buddy"
    );
    assert!(
        allocator.free_lists[1].is_some(),
        "Order 1 should have the split buddy"
    );
    assert!(
        allocator.free_lists[2].is_some(),
        "Order 2 should have the split buddy"
    );
    assert!(
        allocator.free_lists[3].is_some(),
        "Order 3 should have the split buddy"
    );

    let addr2 = allocator
        .alloc_pages(0)
        .expect("Should allocate second order 0 page");
    assert_eq!(
        addr2,
        start + PAGE_SIZE,
        "Second allocation should be next page"
    );

    // 2. Test higher order allocation (order 2, size 16KB)
    let addr3 = allocator
        .alloc_pages(2)
        .expect("Should allocate order 2 block");
    assert_eq!(
        addr3,
        start + 4 * PAGE_SIZE,
        "Order 2 block should be at start + 16KB"
    );

    // 3. Test freeing and merging back to the original order 4 block
    allocator.free_pages(addr1, 0);
    // After freeing addr1, buddy addr2 is not free, so it is just inserted in order 0
    assert!(allocator.free_lists[0].is_some());

    allocator.free_pages(addr2, 0);
    // After freeing addr2, it merges with addr1 to form order 1 block at start.
    // That merges with order 1 block at start + 8KB to form order 2 block at start.
    // Buddy of order 2 block at start is start + 16KB, which is addr3 (allocated), so it stops merging.
    // Order 2 list should now contain the merged block at start.

    allocator.free_pages(addr3, 2);
    // Freeing addr3 (start + 16KB) merges with order 2 block at start to form order 3 block at start.
    // That merges with order 3 block at start + 32KB to form order 4 block at start.
    // The allocator should return to its original single order 4 block state.

    for i in 0..4 {
        assert!(
            allocator.free_lists[i].is_none(),
            "Free list {} should be empty after full merge",
            i
        );
    }
    assert!(
        allocator.free_lists[4].is_some(),
        "Free list 4 should have the merged block"
    );

    // 4. Test allocation after merging
    let addr4 = allocator
        .alloc_pages(3)
        .expect("Should allocate order 3 block (32KB) after merge");
    assert_eq!(addr4, start, "Order 3 block should be at start");
    allocator.free_pages(addr4, 3);

    // boot-time test end
}

// ---------------------------------------------------------------------------
// Unit tests — runnable on any host with `cargo test --lib`
//
// Design constraints
// ==================
// 1. `PageAllocatorState` is a plain Rust struct with no MMIO or platform
//    registers.  It can be constructed with `PageAllocatorState::new()` and
//    initialised with `state.init(start, end)` entirely in user-space.
//
// 2. The module is compiled with `#![no_std]` for the kernel binary, but
//    `#[cfg(test)]` flips the build target to the host (x86-64 / aarch64)
//    where `std` is available.  `core::ptr` works identically in both.
//
// 3. Each test heap-allocates its 64 KB test region via `Box` so the test
//    thread stack (default 8 MB on Linux/macOS) is never stressed.
//    The `Box` pointer is kept alive for the duration of the test via a
//    binding (`_buf`) so the allocator's interior pointers stay valid.
//
// 4. Tests that exercise `PageAllocatorState::alloc_pages` /
//    `free_pages` use `unsafe` blocks because the methods write raw bytes
//    into the test buffer.  Each `unsafe` block carries a SAFETY comment.
//
// 5. The global `ALLOCATOR` (wrapped in `spin::Mutex`) is intentionally
//    NOT touched from tests — doing so would cause test ordering to matter
//    and would require platform initialisation that isn't available on the
//    host.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // Helper: build a fresh PageAllocatorState backed by a heap buffer.
    //
    // Returns `(state, _buf)` where `_buf` must be kept in scope for the
    // duration of the test to prevent the Box from being dropped while
    // `state` holds interior pointers into it.
    //
    // The buffer is 64 KB and aligned to 64 KB.  A 64 KB-aligned region
    // is exactly one order-4 block (16 pages × 4 KB), which gives tests
    // a clean, predictable free-list.
    // ------------------------------------------------------------------
    fn make_allocator() -> (PageAllocatorState, Box<[u8]>) {
        // Allocate raw bytes.  We need 64 KB *and* 64 KB alignment so that
        // the buddy maths are clean.  `Box<[u8]>` from `vec!` is 1-aligned,
        // so we allocate 128 KB and find the aligned window inside it.
        let raw: Box<[u8]> = vec![0u8; 128 * 1024].into_boxed_slice();
        let base = raw.as_ptr() as usize;
        // Round up to the next 64 KB boundary.
        let align = 64 * 1024;
        let start = (base + align - 1) & !(align - 1);
        let end = start + align;

        // `end` is guaranteed to be within the allocation because
        // raw.len() == 128 KB and start <= base + align - 1 < base + align.
        assert!(end <= base + raw.len(), "alignment window overflows buffer");

        let mut state = PageAllocatorState::new();
        state.init(start, end);
        (state, raw)
    }

    // ------------------------------------------------------------------
    // Test 1: order_for_size — verify the order <-> byte-size relationship.
    //
    // The buddy allocator uses `order` as an exponent: a block of order N
    // holds 2^N pages, each PAGE_SIZE bytes.  This test validates the
    // inverse: given a desired byte size, the correct order is chosen.
    //
    // The allocator does not expose an explicit `order_for_size` function —
    // callers compute the order themselves.  This test documents and pins
    // the expected relationship so any future helper function can be
    // validated against it.
    // ------------------------------------------------------------------
    #[test]
    fn test_order_for_size() {
        // Helper that mirrors the caller-side computation.
        fn order_for_size(bytes: usize) -> usize {
            let pages = bytes / PAGE_SIZE;
            // Find the smallest order such that 2^order >= pages.
            let mut order = 0usize;
            while (1usize << order) < pages {
                order += 1;
            }
            order
        }

        assert_eq!(order_for_size(4096), 0, "4 KB  → order 0");
        assert_eq!(order_for_size(8192), 1, "8 KB  → order 1");
        assert_eq!(order_for_size(16384), 2, "16 KB → order 2");
        assert_eq!(order_for_size(32768), 3, "32 KB → order 3");
        assert_eq!(order_for_size(65536), 4, "64 KB → order 4");
        assert_eq!(order_for_size(1024 * 1024), 8, "1 MB  → order 8");
        assert_eq!(order_for_size(4 * 1024 * 1024), 10, "4 MB  → order 10 (max)");
    }

    // ------------------------------------------------------------------
    // Test 2: NAPOT alignment invariant.
    //
    // PMP NAPOT encoding requires that a region of size S starts at an
    // address that is a multiple of S.  The buddy allocator enforces this
    // implicitly: a block returned at order N has size (2^N * PAGE_SIZE)
    // and is always aligned to that size because:
    //   - init() rounds the start address UP to PAGE_SIZE,
    //   - splitting always places the lower half at the parent's address
    //     (which is already aligned to the larger size), and
    //   - the buddy address is computed as `addr ^ block_size`, which
    //     flips the bit that distinguishes the two halves.
    //
    // This test directly checks that addresses returned by alloc_pages()
    // satisfy the NAPOT alignment property for their order.
    // ------------------------------------------------------------------
    #[test]
    fn test_napot_alignment() {
        let (mut state, _buf) = make_allocator();

        // Allocate blocks at every order from 0..=4 (our 64 KB heap
        // can satisfy at most one order-4, two order-3, …, sixteen order-0
        // allocations without overlap; we just need one each).
        for order in 0..=4usize {
            // Re-init for each sub-test so we always start fresh.
            let (mut fresh, _b) = make_allocator();
            let block_size = (1usize << order) * PAGE_SIZE;

            let addr = fresh.alloc_pages(order).expect("allocation must succeed");

            assert_eq!(
                addr % block_size,
                0,
                "order-{order} allocation at 0x{addr:x} must be {block_size}-byte aligned (NAPOT)"
            );
        }
    }

    // ------------------------------------------------------------------
    // Test 3: alloc + free roundtrip — the same address must be reusable.
    //
    // After freeing an order-0 page, the next order-0 allocation from a
    // fresh free-list must return the same address (because the buddy
    // merging restores the pool to its initial state and the list head
    // points at the same block).
    // ------------------------------------------------------------------
    #[test]
    fn test_alloc_free_roundtrip() {
        let (mut state, _buf) = make_allocator();

        let addr1 = state.alloc_pages(0).expect("first alloc must succeed");

        // Free it back.
        state.free_pages(addr1, 0);

        // All buddies should merge back to the single order-4 block.
        // A subsequent order-0 allocation must re-use the same address.
        let addr2 = state.alloc_pages(0).expect("second alloc must succeed after free");

        assert_eq!(
            addr1, addr2,
            "address after free/re-alloc must be identical (buddy fully merged)"
        );

        // Cleanup — free so we don't leave the test heap in a dirty state
        // (matters if the test framework reuses address space).
        state.free_pages(addr2, 0);
    }

    // ------------------------------------------------------------------
    // Test 4: double-free detection.
    //
    // `free_pages` asserts that `addr >= start_addr && addr < end_addr`.
    // A double-free means the address is still inside the valid range, so
    // the bounds assertion alone is insufficient.
    //
    // The allocator handles double-free via the buddy XOR merge loop: if a
    // page is freed twice the second free will try to find its buddy in the
    // free list.  After the first free the buddy block has been merged
    // upward, so `remove_block` returns false and the block is simply
    // pushed onto the list a second time — creating a duplicate entry.
    //
    // This test verifies the *current* behaviour (no explicit panic) and
    // documents it as a known limitation.  A future improvement would add
    // a use-after-free bitmap, but that requires O(heap/PAGE_SIZE) metadata.
    //
    // NOTE: The test is marked `#[should_panic]` only if we can guarantee
    // a panic.  Since the current implementation does NOT panic on double-
    // free (it silently corrupts the free list), we instead assert the
    // observable consequence: two allocations after a double-free both
    // return the same address, proving the duplicate list entry exists.
    // ------------------------------------------------------------------
    #[test]
    fn test_double_free_detection() {
        // This test documents that the current allocator does NOT detect
        // double-free at runtime — the free list silently gets a duplicate
        // entry.  The symptom is that two subsequent allocations return the
        // same physical address, which would cause memory aliasing bugs in
        // production.
        //
        // Mitigation: callers must use capability-layer ownership tracking
        // (CapabilityEntry in capability.rs) to prevent double-free rather
        // than relying on allocator-level detection.

        let (mut state, _buf) = make_allocator();

        let addr = state.alloc_pages(0).expect("alloc must succeed");

        // First free — legitimate.
        state.free_pages(addr, 0);

        // Second free of the same address — this is the double-free.
        // The allocator does not panic; it pushes the block onto the list
        // a second time.
        state.free_pages(addr, 0);

        // Consequence: both subsequent allocations return the same address.
        let a = state.alloc_pages(0).expect("first alloc after double-free");
        let b = state.alloc_pages(0).expect("second alloc after double-free");

        assert_eq!(
            a, b,
            "double-free creates duplicate free-list entry: both allocations aliased to 0x{a:x}"
        );
        // Document the known issue rather than hiding it.
        // A future patch should add a freed-page bitmap or per-page metadata.
    }

    // ------------------------------------------------------------------
    // Test 5: order-boundary correctness.
    //
    // Verify that pages adjacent to an order boundary are allocated and
    // freed independently without cross-contamination.  Specifically:
    //
    //   - Allocate two adjacent order-0 pages: addr_lo and addr_hi.
    //   - They must be PAGE_SIZE apart.
    //   - Free addr_lo first; the buddy (addr_hi) is still allocated, so
    //     no merge should happen → order-0 list gets one entry.
    //   - Free addr_hi; the buddy (addr_lo) is now free, so they merge
    //     into one order-1 block → order-0 list becomes empty, order-1
    //     list gets an entry.
    //   - Continue freeing to verify full collapse back to order-4.
    // ------------------------------------------------------------------
    #[test]
    fn test_order_boundary() {
        let (mut state, _buf) = make_allocator();

        // After init the heap is a single order-4 block at `start`.
        // Verify the expected initial free-list shape.
        for i in 0..4 {
            assert!(state.free_lists[i].is_none(), "order {i} should be empty at init");
        }
        assert!(state.free_lists[4].is_some(), "order 4 should have the 64 KB block");

        let addr_lo = state.alloc_pages(0).expect("alloc order-0 block lo");
        let addr_hi = state.alloc_pages(0).expect("alloc order-0 block hi");

        assert_eq!(
            addr_hi,
            addr_lo + PAGE_SIZE,
            "adjacent order-0 allocations must be exactly PAGE_SIZE apart"
        );

        // Free the lower page only.  Its buddy (addr_hi) is still allocated,
        // so no merge: order-0 list gets one entry.
        state.free_pages(addr_lo, 0);
        assert!(
            state.free_lists[0].is_some(),
            "order-0 list must hold addr_lo after partial free"
        );
        assert!(
            state.free_lists[1].is_none(),
            "order-1 list must remain empty — buddy addr_hi is still allocated"
        );

        // Free the upper page.  Now both buddies are free → they merge to
        // order-1.  The order-1 buddy (addr_lo+2×PAGE_SIZE) was placed on
        // order-1 when the original order-4 block was split, so they merge
        // further, and so on until the full order-4 block is restored.
        state.free_pages(addr_hi, 0);

        // Full merge: all orders 0..=3 empty, order 4 has the block.
        for i in 0..4 {
            assert!(
                state.free_lists[i].is_none(),
                "order {i} must be empty after full buddy merge"
            );
        }
        assert!(
            state.free_lists[4].is_some(),
            "order 4 must hold the merged 64 KB block"
        );
    }
}
