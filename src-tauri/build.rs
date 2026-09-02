fn main() {
    tauri_build::build();

    // Locate FreeRDP + WinPR via pkg-config (dev: brew). `probe_library` also
    // emits the required link-search/-lib cargo directives.
    let freerdp3 = pkg_config::probe_library("freerdp3")
        .expect("freerdp3 (brew freerdp) required for embedded desktop");
    let client3 = pkg_config::probe_library("freerdp-client3").expect("freerdp-client3 required");
    let winpr3 = pkg_config::probe_library("winpr3").expect("winpr3 required");

    let mut includes = Vec::new();
    for lib in [&freerdp3, &client3, &winpr3] {
        includes.extend(lib.include_paths.iter().cloned());
    }

    // Absolute paths so cargo's change-tracking on the native sources is
    // reliable (relative `../` paths are intermittently missed by cc/cargo).
    let native = std::path::Path::new("../native/freerdp_bridge")
        .canonicalize()
        .expect("native/freerdp_bridge must exist");
    let bridge_c = native.join("bridge.c");
    let bridge_h = native.join("bridge.h");
    let macos_view_m = native.join("macos_view.m");
    // The header isn't a compiled input, so track its changes explicitly.
    println!("cargo:rerun-if-changed={}", bridge_h.display());
    println!("cargo:rerun-if-changed={}", bridge_c.display());
    println!("cargo:rerun-if-changed={}", macos_view_m.display());

    // C bridge (session protocol) — no ARC.
    let mut bridge = cc::Build::new();
    bridge.file(&bridge_c);
    bridge.flag("-Wno-deprecated-declarations");
    bridge.flag("-Wno-unused-parameter");
    for inc in &includes {
        bridge.include(inc);
    }
    bridge.compile("freerdp_bridge_c");

    // ObjC native view — ARC; AppKit on the main thread.
    let mut view = cc::Build::new();
    view.file(&macos_view_m);
    view.flag("-fobjc-arc");
    view.flag("-Wno-deprecated-declarations");
    for inc in &includes {
        view.include(inc);
    }
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=CoreGraphics");
    println!("cargo:rustc-link-lib=framework=QuartzCore");
    view.compile("freerdp_bridge_view");
}
