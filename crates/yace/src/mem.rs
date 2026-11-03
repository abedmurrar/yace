//! Returning memory to the OS (glibc only).
//!
//! glibc gives every thread its own malloc arena and keeps freed memory in
//! them. After a parallel walk or search the process would otherwise keep
//! tens of MiB it no longer uses. We cap the number of arenas at startup
//! and trim after background work finishes.
//!
//! This is the only module in YACE that uses `unsafe`: two FFI calls into
//! glibc with no pointers involved.

#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[allow(unsafe_code)]
mod imp {
    /// `YACE_MALLOC_TRIM=0` disables the tuning (for A/B measurements).
    fn disabled() -> bool {
        std::env::var_os("YACE_MALLOC_TRIM").is_some_and(|v| v == "0")
    }

    /// Limit glibc to two malloc arenas (call before spawning threads).
    pub fn limit_arenas() {
        if disabled() {
            return;
        }
        // SAFETY: mallopt only reads its integer arguments; it is called
        // once at startup before other threads exist.
        unsafe {
            libc::mallopt(libc::M_ARENA_MAX, 2);
        }
    }

    /// Release free heap memory back to the OS.
    pub fn trim() {
        if disabled() {
            return;
        }
        // SAFETY: malloc_trim takes an integer and touches only allocator
        // state, which is internally synchronised by glibc.
        unsafe {
            libc::malloc_trim(0);
        }
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
mod imp {
    pub fn limit_arenas() {}
    pub fn trim() {}
}

pub use imp::{limit_arenas, trim};
