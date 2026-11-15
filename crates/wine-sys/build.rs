fn main() {
    // Only link when the `link` feature is requested, so `cargo test` works
    // on machines without Wine development libraries.
    let linking = std::env::var("CARGO_FEATURE_LINK").is_ok();
    if !linking {
        println!("cargo:rustc-cfg=not_linked");
        println!("cargo:rerun-if-changed=build.rs");
        return;
    }
    match pkgconfig_probe() {
        Some((_, lib)) => {
            println!("cargo:rustc-link-search=native={lib}");
            println!("cargo:rustc-link-lib=dylib=wine");
            println!("cargo:rustc-cfg=linked_libwine");
        }
        None => {
            println!("cargo:warning=libwine not found via pkg-config; calls will fail at link time");
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
}

fn pkgconfig_probe() -> Option<(String, String)> {
    let out = std::process::Command::new("pkg-config")
        .args(["--cflags-only-I", "--libs-only-L", "wine"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut include = String::new();
    let mut libdir = String::new();
    for tok in text.split_whitespace() {
        if let Some(v) = tok.strip_prefix("-I") {
            include = v.to_string();
        }
        if let Some(v) = tok.strip_prefix("-L") {
            libdir = v.to_string();
        }
    }
    Some((include, libdir))
}
