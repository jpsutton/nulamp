//! Links libopenmpt for the `openmpt` feature and libcdio for the `cdio` feature. When a library
//! cannot be found, the decoder that needs it is left out (with a build warning) instead of
//! failing the build.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(nulamp_openmpt)");
    println!("cargo::rustc-check-cfg=cfg(nulamp_cdio)");
    cdio();
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

/// libcdio (GPL-3.0-or-later) is opt-in: the `cdio` feature is off by default
/// (docs/decisions.md CD1).
fn cdio() {
    println!("cargo:rerun-if-env-changed=CDIO_LIB_DIR");
    if std::env::var_os("CARGO_FEATURE_CDIO").is_none() {
        return;
    }
    if let Some(dir) = std::env::var_os("CDIO_LIB_DIR") {
        println!("cargo:rustc-link-search=native={}", dir.to_string_lossy());
        println!("cargo:rustc-link-lib=cdio");
        println!("cargo:rustc-cfg=nulamp_cdio");
        return;
    }
    match pkg_config::Config::new()
        .atleast_version("2.0")
        .probe("libcdio")
    {
        Ok(_) => println!("cargo:rustc-cfg=nulamp_cdio"),
        Err(e) => println!(
            "cargo:warning=libcdio >= 2.0 not found ({e}); audio CD playback is disabled. \
             Install libcdio or set CDIO_LIB_DIR to enable it."
        ),
    }
}
