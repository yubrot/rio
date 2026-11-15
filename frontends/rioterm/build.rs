fn main() {
    cfg_aliases::cfg_aliases! {
        // rioterm's own `wgpu` feature, or the targets whose
        // target-specific dependency overrides force the feature onto
        // sugarloaf and rio-backend (Windows and wasm32 in Cargo.toml),
        // so the wgpu code paths always exist there.
        wgpu_backend: {
            any(
                feature = "wgpu",
                target_os = "windows",
                target_arch = "wasm32"
            )
        },
    }

    // wgpu-hal references HDR colorspace constants (ExtendedDisplayP3,
    // the ITUR_2100 pair) that older macOS does not export. Their use
    // is runtime-gated inside wgpu, but the references link as strong
    // imports, so dyld refuses to load the binary on macOS 10.15 even
    // though those paths can never run there (the bundle advertises
    // LSMinimumSystemVersion 10.15.7). Weak-linking CoreGraphics turns
    // the missing constants into NULLs instead of a launch abort; rio
    // never requests an HDR surface colorspace, so they stay untouched,
    // and the constants rio does use (DisplayP3) exist since 10.11.
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("apple-darwin") {
        println!("cargo:rustc-link-arg-bins=-weak_framework");
        println!("cargo:rustc-link-arg-bins=CoreGraphics");
    }

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_app_icon();
    }
}

// The exe resource Explorer and the taskbar read; the same image the
// runtime window icon uses (router::window::LOGO_ICON). embed-resource
// uses rc.exe on Windows hosts and llvm-rc when cross-compiling.
fn embed_app_icon() {
    println!("cargo:rerun-if-changed=src/router/resources/images/rio-logo.ico");
    embed_resource::compile("rio.rc", embed_resource::NONE)
        .manifest_required()
        .expect("failed to compile the windows icon resource");
}
