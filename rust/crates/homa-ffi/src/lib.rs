//! Stable C ABI over [`homa_rs`] - **Phase 2 placeholder**.
//!
//! Phase 1 deliberately does not implement the managed interop surface. This
//! crate exists so the workspace layout, build type (`cdylib`/`staticlib`) and
//! CI are in place for Phase 2, where it will expose a flat, versioned
//! `extern "C"` API consumed by `Homa.NET` through `LibraryImport`.
//!
//! See `docs/native-interop.md` for the planned surface and ownership rules.

/// ABI version reported to managed callers. Bumped on any breaking change to
/// the exported C surface.
pub const HOMA_FFI_ABI_VERSION: u32 = 0;

/// Returns [`HOMA_FFI_ABI_VERSION`]. The only export in Phase 1; lets a
/// managed host verify it loaded a compatible native library.
#[unsafe(no_mangle)]
pub extern "C" fn homa_ffi_abi_version() -> u32 {
    HOMA_FFI_ABI_VERSION
}

/// Whether the native library was built for a platform where Homa sockets can
/// exist (Linux). Managed code can fail fast elsewhere.
#[unsafe(no_mangle)]
pub extern "C" fn homa_ffi_platform_supported() -> bool {
    homa_rs::is_supported_platform()
}

#[cfg(test)]
mod tests {
    #[test]
    fn abi_version_is_stable() {
        assert_eq!(super::homa_ffi_abi_version(), 0);
    }
}
