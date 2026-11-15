//! Build script: links `libfex_wine` when the `link` feature is on.

fn main() {
    let linking = std::env::var("CARGO_FEATURE_LINK").is_ok();
    if linking {
        // fex_wine is built inside Valve's SteamOS wine build; point
        // FEX_WINE_LIB at the directory holding libfex_wine.so.
        if let Ok(dir) = std::env::var("FEX_WINE_LIB") {
            println!("cargo:rustc-link-search=native={dir}");
            println!("cargo:rustc-link-lib=dylib=fex_wine");
            println!("cargo:rustc-cfg=linked_fex_wine");
        } else {
            panic!("FEX_WINE_LIB must be set when the `link` feature is enabled");
        }
    }
    println!("cargo:rerun-if-env-changed=FEX_WINE_LIB");
}
