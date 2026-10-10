//! Links libopenmpt for the `openmpt` feature. When the library cannot be found, the module
//! decoder is left out (with a build warning) instead of failing the build.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(nulamp_openmpt)");
    println!("cargo:rerun-if-env-changed=OPENMPT_LIB_DIR");
    if std::env::var_os("CARGO_FEATURE_OPENMPT").is_none() {
        return;
    }
    if let Some(dir) = std::env::var_os("OPENMPT_LIB_DIR") {
        println!("cargo:rustc-link-search=native={}", dir.to_string_lossy());
        println!("cargo:rustc-link-lib=openmpt");
        println!("cargo:rustc-cfg=nulamp_openmpt");
        return;
    }
    match pkg_config::Config::new()
        .atleast_version("0.5")
        .probe("libopenmpt")
    {
        Ok(_) => println!("cargo:rustc-cfg=nulamp_openmpt"),
        Err(e) => println!(
            "cargo:warning=libopenmpt >= 0.5 not found ({e}); tracker modules are disabled. \
             Install libopenmpt or set OPENMPT_LIB_DIR to enable them."
        ),
    }
}
