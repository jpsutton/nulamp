//! Links the optional native libraries:
//!
//! - libopenmpt for the `openmpt` feature (tracker modules);
//! - FFmpeg (libavformat, libavcodec, libavutil, libswresample) for the `ffmpeg` feature, through
//!   a small C shim (`ffmpeg/shim.c`) compiled against the installed headers.
//!
//! When a library cannot be found, its decoder is left out (with a build warning) instead of
//! failing the build.

use std::path::PathBuf;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(nulamp_openmpt)");
    println!("cargo::rustc-check-cfg=cfg(nulamp_ffmpeg)");
    openmpt();
    ffmpeg();
}

fn openmpt() {
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

/// FFmpeg libraries and the oldest versions with the APIs the shim uses (FFmpeg 5.1:
/// `AVChannelLayout`, `swr_alloc_set_opts2`).
const FFMPEG_LIBS: &[(&str, &str, &str)] = &[
    ("libavformat", "avformat", "59.27"),
    ("libavcodec", "avcodec", "59.37"),
    ("libavutil", "avutil", "57.28"),
    ("libswresample", "swresample", "4.7"),
];

fn ffmpeg() {
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");
    println!("cargo:rerun-if-changed=ffmpeg/shim.c");
    if std::env::var_os("CARGO_FEATURE_FFMPEG").is_none() {
        return;
    }
    // FFMPEG_DIR: a shared FFmpeg build with `include/` and `lib/` (Windows, custom builds).
    let (includes, link_dirs, libs) = if let Some(dir) = std::env::var_os("FFMPEG_DIR") {
        let dir = PathBuf::from(dir);
        let libs = FFMPEG_LIBS.iter().map(|(_, l, _)| l.to_string()).collect();
        (vec![dir.join("include")], vec![dir.join("lib")], libs)
    } else {
        let (mut includes, mut link_dirs, mut libs) = (Vec::new(), Vec::new(), Vec::new());
        for (name, _, version) in FFMPEG_LIBS {
            match pkg_config::Config::new()
                .atleast_version(version)
                .statik(false)
                .cargo_metadata(false)
                .probe(name)
            {
                Ok(lib) => {
                    includes.extend(lib.include_paths);
                    link_dirs.extend(lib.link_paths);
                    libs.extend(lib.libs);
                }
                Err(e) => {
                    println!(
                        "cargo:warning={name} >= {version} not found ({e}); the FFmpeg decoder \
                         (WMA, WavPack, Monkey's Audio, Musepack, TTA) is disabled. Install \
                         the FFmpeg development libraries or set FFMPEG_DIR to enable it."
                    );
                    return;
                }
            }
        }
        (includes, link_dirs, libs)
    };
    let mut build = cc::Build::new();
    build.file("ffmpeg/shim.c").warnings(true);
    for dir in &includes {
        build.include(dir);
    }
    // The shim (a static library) is linked before the shared FFmpeg libraries it uses.
    if let Err(e) = build.try_compile("nulamp_ffmpeg_shim") {
        println!(
            "cargo:warning=the FFmpeg shim did not compile ({e}); the FFmpeg decoder is disabled."
        );
        return;
    }
    for dir in link_dirs {
        println!("cargo:rustc-link-search=native={}", dir.display());
    }
    let mut seen = Vec::new();
    for lib in libs {
        if !seen.contains(&lib) {
            println!("cargo:rustc-link-lib=dylib={lib}");
            seen.push(lib);
        }
    }
    println!("cargo:rustc-cfg=nulamp_ffmpeg");
}
