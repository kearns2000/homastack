//! Build script: exposes the vendored header directory to dependents and to
//! the optional `header-check` test (via `DEP_HOMA_UAPI_INCLUDE` / env).

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set");
    let include = format!("{manifest_dir}/upstream");
    println!("cargo:include={include}");
    println!("cargo:rustc-env=HOMA_SYS_UPSTREAM_INCLUDE={include}");
    println!("cargo:rerun-if-changed=upstream/homa.h");
    println!("cargo:rerun-if-changed=build.rs");
}
