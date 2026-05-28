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
pub fn test_page_alloc() {
    crate::println!("[TEST] Running Buddy Allocator Unit Tests...");

    #[allow(dead_code)]
    #[repr(align(65536))]
    struct TestBuffer([u8; 64 * 1024]); // 64KB buffer aligned to 64KB (order 4 size)
    static mut TEST_BUF: TestBuffer = TestBuffer([0; 64 * 1024]);

    let start = core::ptr::addr_of_mut!(TEST_BUF) as usize;
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

    crate::println!("[TEST] Buddy Allocator Unit Tests Passed successfully!");
}
