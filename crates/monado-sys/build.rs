fn main() {
    let linking = std::env::var("CARGO_FEATURE_LINK").is_ok();
    if linking {
        println!("cargo:rustc-link-lib=dylib=openxr_loader");
        println!("cargo:rustc-cfg=linked_openxr_loader");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
