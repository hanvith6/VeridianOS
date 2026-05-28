//! InitRAMFS: POSIX ustar TAR Archive Parser for VeridianOS
//!
//! Reads a POSIX ustar TAR archive from the VirtIO block device into a
//! static kernel memory buffer, then provides fast by-name file lookup.
//!
//! # Why ustar TAR?
//!
//! - Standard POSIX `tar cf disk.img file1 file2` creates a compatible image
//! - Fixed 512-byte header blocks make zero-allocation parsing trivial
//! - No external crates needed — fully `no_std` compatible
//!
//! # TAR Format Summary (ustar)
//!
//! A TAR archive is a sequence of 512-byte blocks:
//! - **Header block**: Describes the next file (name, size, type, checksum)
//! - **Data blocks**: The file content (padded to next 512-byte boundary)
//! - **End-of-archive**: Two consecutive all-zero blocks
//!
//! References:
//! - IEEE Std 1003.1 (POSIX) ustar format specification
//! - GNU tar manual §8

use crate::virtio::blk::{self, SECTOR_SIZE};
use spin::Mutex;

/// Maximum size of the RAM buffer used to hold the entire disk image.
/// Set to 4 MB = 8192 sectors. Enough for a typical initramfs.
const RAMFS_BUF_SIZE: usize = 4 * 1024 * 1024;

/// Maximum number of files the ramfs index can track.
const MAX_FILES: usize = 64;

/// A ustar TAR header block (exactly 512 bytes).
///
/// The header precedes each file in the archive. All numeric fields
/// are stored as ASCII octal strings (null-terminated).
#[repr(C)]
struct UstarHeader {
    /// File name (null-terminated string, up to 100 chars)
    name: [u8; 100],
    /// File mode (octal ASCII)
    mode: [u8; 8],
    /// Owner user ID (octal ASCII)
    uid: [u8; 8],
    /// Owner group ID (octal ASCII)
    gid: [u8; 8],
    /// File size in bytes (octal ASCII, 12 chars)
    size: [u8; 12],
    /// Last modification time (octal ASCII)
    mtime: [u8; 12],
    /// Header checksum
    checksum: [u8; 8],
    /// Link indicator / file type: '0'=regular, '5'=directory, etc.
    typeflag: u8,
    /// Link target name (for symlinks)
    linkname: [u8; 100],
    /// "ustar" magic string — identifies this as a POSIX ustar archive
    magic: [u8; 6],
    /// ustar version ("00")
    version: [u8; 2],
    /// Owner username string
    uname: [u8; 32],
    /// Owner groupname string
    gname: [u8; 32],
    /// Device major number
    devmajor: [u8; 8],
    /// Device minor number
    devminor: [u8; 8],
    /// Filename prefix (prepended to `name` with `/` separator for long paths)
    prefix: [u8; 155],
    /// Padding to reach 512 bytes
    _padding: [u8; 12],
}

/// A parsed file entry stored in the RamFs index.
#[derive(Clone, Copy)]
struct FileEntry {
    /// Null-padded filename (up to 100 chars)
    name: [u8; 100],
    /// Byte offset into `RAMFS_BUF` where the file's data starts
    data_offset: usize,
    /// Length of the file data in bytes
    data_len: usize,
}

/// The complete in-kernel RamFs state.
struct RamFsState {
    /// Raw bytes loaded from the disk (up to RAMFS_BUF_SIZE)
    buf: [u8; RAMFS_BUF_SIZE],
    /// Number of bytes actually loaded from disk
    loaded_bytes: usize,
    /// Parsed file index
    files: [Option<FileEntry>; MAX_FILES],
    /// Number of files found in the archive
    file_count: usize,
    /// Whether init has been called
    initialized: bool,
}

impl RamFsState {
    const fn new() -> Self {
        Self {
            buf: [0u8; RAMFS_BUF_SIZE],
            loaded_bytes: 0,
            files: [None; MAX_FILES],
            file_count: 0,
            initialized: false,
        }
    }
}

static RAMFS: Mutex<RamFsState> = Mutex::new(RamFsState::new());

/// Parse an ASCII octal string (as stored in ustar headers) into a `usize`.
///
/// TAR stores numeric fields as null-terminated or space-terminated ASCII
/// octal strings. This function handles both terminators.
fn parse_octal(bytes: &[u8]) -> usize {
    let mut result = 0usize;
    for &b in bytes {
        if b == 0 || b == b' ' {
            break;
        }
        if (b'0'..=b'7').contains(&b) {
            result = result * 8 + (b - b'0') as usize;
        }
    }
    result
}

/// Copy a null-terminated byte slice into a fixed-size array.
fn copy_name(src: &[u8; 100], dst: &mut [u8; 100]) {
    dst.copy_from_slice(src);
}

/// Compare a null-terminated name array against a &str.
fn name_matches(entry_name: &[u8; 100], query: &str) -> bool {
    let query_bytes = query.as_bytes();
    // Compare byte-by-byte up to the length of the query
    for (i, &qb) in query_bytes.iter().enumerate() {
        if i >= 100 || entry_name[i] != qb {
            return false;
        }
    }
    // The next char in entry_name should be null (end of string) or we've consumed all query bytes
    if query_bytes.len() < 100 {
        let next = entry_name[query_bytes.len()];
        return next == 0 || next == b' ';
    }
    true
}

/// Public handle to the RamFs — a zero-sized type whose methods
/// acquire the global lock internally.
pub struct RamFs;

impl RamFs {
    /// Load the disk image from the VirtIO block device and parse the ustar archive.
    ///
    /// Must be called once at kernel boot after `virtio::blk::init()` succeeds.
    pub fn load_from_disk() -> Result<usize, &'static str> {
        let mut state = RAMFS.lock();

        let total_sectors = blk::capacity();
        if total_sectors == 0 {
            return Err("RamFs: disk has zero sectors");
        }

        // Cap to our buffer size
        let sectors_to_read = (RAMFS_BUF_SIZE / SECTOR_SIZE).min(total_sectors as usize);

        crate::println!(
            "[RAMFS] Reading {} sectors ({} KB) from disk...",
            sectors_to_read,
            sectors_to_read / 2
        );

        // Read sectors directly into the buffer
        let buf_slice = &mut state.buf[..sectors_to_read * SECTOR_SIZE];
        blk::read_sectors(0, sectors_to_read, buf_slice)?;
        state.loaded_bytes = sectors_to_read * SECTOR_SIZE;

        // Parse the ustar TAR archive
        let mut offset = 0usize;
        let mut file_count = 0;

        while offset + 512 <= state.loaded_bytes {
            // Safety: buf is valid, offset is aligned to 512-byte blocks
            let header = unsafe {
                &*(state.buf.as_ptr().add(offset) as *const UstarHeader)
            };

            // Check for end-of-archive marker (two all-zero blocks)
            if header.name[0] == 0 {
                break;
            }

            // Verify "ustar" magic (bytes 257-262)
            let magic = &header.magic;
            let is_ustar = magic[0] == b'u'
                && magic[1] == b's'
                && magic[2] == b't'
                && magic[3] == b'a'
                && magic[4] == b'r';

            // Parse file size from octal ASCII
            let file_size = parse_octal(&header.size);
            let data_offset = offset + 512; // data starts right after the header block

            // Only index regular files (typeflag '0' or '\0')
            let is_regular = header.typeflag == b'0' || header.typeflag == 0;

            if is_ustar && is_regular && file_count < MAX_FILES {
                let mut entry = FileEntry {
                    name: [0u8; 100],
                    data_offset,
                    data_len: file_size,
                };
                copy_name(&header.name, &mut entry.name);
                state.files[file_count] = Some(entry);
                file_count += 1;
            }

            // Advance past header block + data blocks (rounded up to 512-byte boundary)
            let data_blocks = file_size.div_ceil(512);
            offset += 512 + data_blocks * 512;
        }

        state.file_count = file_count;
        state.initialized = true;

        crate::println!("[RAMFS] Parsed archive: {} file(s) found:", file_count);
        for i in 0..file_count {
            if let Some(ref entry) = state.files[i] {
                // Print just the non-null bytes of the name
                let name_len = entry.name.iter().position(|&b| b == 0).unwrap_or(100);
                let name_str = core::str::from_utf8(&entry.name[..name_len])
                    .unwrap_or("<invalid utf8>");
                crate::println!(
                    "  [{:02}] {:32} ({} bytes)",
                    i, name_str, entry.data_len
                );
            }
        }

        Ok(file_count)
    }

    /// Look up a file in the RamFs by name.
    ///
    /// Returns a byte slice into the static RAM buffer if found,
    /// or `None` if the file doesn't exist.
    ///
    /// # Safety
    ///
    /// The returned slice is a raw pointer into static memory — it is valid
    /// for the lifetime of the kernel (never freed or moved).
    pub fn find(name: &str) -> Option<&'static [u8]> {
        let state = RAMFS.lock();

        for i in 0..state.file_count {
            if let Some(ref entry) = state.files[i]
                && name_matches(&entry.name, name) {
                    let start = entry.data_offset;
                    let end = start + entry.data_len;
                    if end <= state.loaded_bytes {
                        // Safety: we return a pointer to static kernel memory
                        let ptr = unsafe { state.buf.as_ptr().add(start) };
                        return Some(unsafe { core::slice::from_raw_parts(ptr, entry.data_len) });
                    }
                }
        }
        None
    }

    /// Returns how many files are in the RamFs.
    pub fn file_count() -> usize {
        RAMFS.lock().file_count
    }
}
