//! A heap buffer aligned to an arbitrary power-of-two boundary.
//!
//! Direct/unbuffered I/O (Windows `FILE_FLAG_NO_BUFFERING`, Linux `O_DIRECT`)
//! requires the read buffer's *address*, not just its length, to be aligned
//! to the device's physical sector size. A plain `Vec<u8>` gives no such
//! guarantee, so we allocate manually.

use std::alloc::{alloc, alloc_zeroed, dealloc, Layout};
use std::ops::{Deref, DerefMut};

pub struct AlignedBuf {
    ptr: *mut u8,
    len: usize,
    layout: Layout,
}

// Safety: AlignedBuf owns its allocation exclusively, like a Vec<u8>.
unsafe impl Send for AlignedBuf {}

impl AlignedBuf {
    /// Allocates `len` zeroed bytes aligned to `align` (must be a power of two).
    pub fn new(len: usize, align: usize) -> Self {
        assert!(align.is_power_of_two(), "alignment must be a power of two");
        assert!(len > 0, "AlignedBuf length must be nonzero");
        let layout = Layout::from_size_align(len, align).expect("invalid layout");
        // Safety: layout has nonzero size.
        let ptr = unsafe { alloc_zeroed(layout) };
        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        Self { ptr, len, layout }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.ptr
    }

    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.ptr
    }
}

impl Deref for AlignedBuf {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        // Safety: ptr is valid for `len` bytes for the lifetime of self.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl DerefMut for AlignedBuf {
    fn deref_mut(&mut self) -> &mut [u8] {
        // Safety: ptr is valid for `len` bytes for the lifetime of self.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        // Safety: ptr/layout match the allocation made in `new`.
        unsafe { dealloc(self.ptr, self.layout) };
    }
}

#[allow(dead_code)]
fn _unused_alloc_import_guard() {
    // keep `alloc` import used on all platforms without a cfg dance
    let _ = alloc as unsafe fn(Layout) -> *mut u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocates_aligned_and_zeroed() {
        let buf = AlignedBuf::new(4096, 4096);
        assert_eq!(buf.len(), 4096);
        assert_eq!(buf.as_ptr() as usize % 4096, 0);
        assert!(buf.iter().all(|&b| b == 0));
    }

    #[test]
    fn writable_via_deref_mut() {
        let mut buf = AlignedBuf::new(512, 512);
        buf[0] = 0xAB;
        buf[511] = 0xCD;
        assert_eq!(buf[0], 0xAB);
        assert_eq!(buf[511], 0xCD);
    }
}
