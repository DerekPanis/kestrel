// Makes the kbridge shared library loadable by the probe binary:
// rpath on Linux/macOS; on Windows, copy kbridge.dll (and any DLLs that sit
// beside it, e.g. vcpkg's) next to the executable.
use std::env;
use std::path::PathBuf;

fn main() {
    let Some(lib_dir) = env::var_os("DEP_KBRIDGE_LIB_DIR").map(PathBuf::from) else {
        return;
    };
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,{}", lib_dir.display());
        return;
    }
    // OUT_DIR = target/<profile>/build/<pkg>-<hash>/out → exe dir is 3 levels up.
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let exe_dir = out.ancestors().nth(3).unwrap().to_path_buf();
    let bin_dir = if lib_dir.join("kbridge.dll").exists() {
        lib_dir.clone()
    } else {
        lib_dir.join("../bin")
    };
    if let Ok(entries) = std::fs::read_dir(&bin_dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dll")) {
                let _ = std::fs::copy(&p, exe_dir.join(p.file_name().unwrap()));
            }
        }
    }
}
