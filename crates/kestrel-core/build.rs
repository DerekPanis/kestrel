// Locates (or builds) the kbridge shared library that wraps libchiaki.
//
// Two modes:
//  * KBRIDGE_LIB_DIR=<dir> — use a kbridge already built by CMake (CI does this,
//    so the heavy C build with vcpkg happens in its own, cacheable step).
//  * otherwise — build ../../native with CMake via the `cmake` crate.

use std::env;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=KBRIDGE_LIB_DIR");
    println!("cargo:rerun-if-changed=../../native/kbridge.c");
    println!("cargo:rerun-if-changed=../../native/kbridge.h");
    println!("cargo:rerun-if-changed=../../native/CMakeLists.txt");

    let lib_dir: PathBuf = match env::var_os("KBRIDGE_LIB_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => build_with_cmake(),
    };

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=dylib=kbridge");
    // Expose the directory to dependents (DEP_KBRIDGE_LIB_DIR) so the app
    // crate can copy the DLL next to its executable.
    println!("cargo:lib_dir={}", lib_dir.display());

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" {
        // Lets `cargo run` / `cargo test` find libkbridge.so without LD_LIBRARY_PATH.
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
    }
}

fn build_with_cmake() -> PathBuf {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let native = manifest.join("../../native");
    let mut cfg = cmake::Config::new(&native);
    cfg.build_target("kbridge").profile("Release");
    if let Some(dir) = env::var_os("CHIAKI_NG_DIR") {
        cfg.define("CHIAKI_NG_DIR", dir);
    }
    let out = cfg.build();
    let build = out.join("build");
    for candidate in [build.join("Release"), build.clone()] {
        if has_lib(&candidate) {
            return candidate;
        }
    }
    panic!(
        "kbridge was built but no library was found under {}",
        build.display()
    );
}

fn has_lib(dir: &Path) -> bool {
    [
        "kbridge.lib",
        "libkbridge.so",
        "libkbridge.dylib",
        "libkbridge.dll.a",
    ]
    .iter()
    .any(|name| dir.join(name).exists())
}
