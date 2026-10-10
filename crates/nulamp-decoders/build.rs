//! Links libopenmpt for the `openmpt` feature.

fn main() {
    println!("cargo:rerun-if-env-changed=OPENMPT_LIB_DIR");
    if std::env::var_os("CARGO_FEATURE_OPENMPT").is_none() {
        return;
    }
    if let Some(dir) = std::env::var_os("OPENMPT_LIB_DIR") {
        println!("cargo:rustc-link-search=native={}", dir.to_string_lossy());
        println!("cargo:rustc-link-lib=openmpt");
        return;
    }
    match pkg_config::Config::new()
        .atleast_version("0.5")
        .probe("libopenmpt")
    {
        Ok(_) => {}
        Err(_) if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") => {
            println!("cargo:rustc-link-lib=openmpt");
        }
        Err(e) => panic!(
            "libopenmpt >= 0.5 not found ({e}). Install it (libopenmpt-dev, brew install \
             libopenmpt), set OPENMPT_LIB_DIR, or build without the `openmpt` feature \
             (--no-default-features)."
        ),
    }
}
