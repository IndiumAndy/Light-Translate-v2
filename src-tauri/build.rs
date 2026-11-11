fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "macos" {
        println!("cargo:rerun-if-changed=native/macos_bridge.m");
        println!("cargo:rerun-if-changed=native/macos_bridge.h");
        cc::Build::new()
            .file("native/macos_bridge.m")
            .flag("-fobjc-arc")
            .flag("-mmacosx-version-min=14.0")
            .flag("-Wno-deprecated-declarations")
            .compile("macos_bridge");

        println!("cargo:rustc-link-lib=framework=Cocoa");
        println!("cargo:rustc-link-lib=framework=ApplicationServices");
        println!("cargo:rustc-link-lib=framework=Vision");
        println!("cargo:rustc-link-lib=framework=ScreenCaptureKit");
        println!("cargo:rustc-link-lib=framework=CoreGraphics");
        println!("cargo:rustc-link-lib=framework=Carbon");
        println!("cargo:rustc-link-lib=framework=QuartzCore");
    }
    tauri_build::build()
}
