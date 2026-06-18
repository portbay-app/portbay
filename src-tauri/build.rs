fn main() {
    // macOS: compile + link the in-process Apple Image Playground bridge
    // (Swift static lib). It has to run inside this app process because Apple's
    // ImageCreator refuses to generate from a background process — see
    // swift/PortBayImagePlayground.swift.
    #[cfg(target_os = "macos")]
    build_image_playground_bridge();

    // Visual editor (Pro): capture OCR uses Apple's Vision framework —
    // linked only when the feature is compiled in, macOS only.
    #[cfg(target_os = "macos")]
    if std::env::var_os("CARGO_FEATURE_VISUAL_EDITOR").is_some() {
        println!("cargo:rustc-link-lib=framework=Vision");
    }
    tauri_build::build()
}

#[cfg(target_os = "macos")]
fn build_image_playground_bridge() {
    use std::path::PathBuf;
    use std::process::Command;

    let src = "swift/PortBayImagePlayground.swift";
    println!("cargo:rerun-if-changed={src}");

    // ImagePlayground only exists in the macOS 15.4+ SDK. On toolchains without
    // it (the public-CI runners), the Swift compiles to stub entry points and
    // we must NOT pass `-framework ImagePlayground` / emit its link directive —
    // a missing framework would otherwise fail the final link. The bridge's C
    // ABI is identical either way, so the Rust side links unchanged.
    let has_image_playground = image_playground_sdk_available();

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "aarch64".into());
    let swift_arch = if arch == "aarch64" {
        "arm64"
    } else {
        arch.as_str()
    };
    // Deployment target matches the app's floor (macOS 14); ImagePlayground is
    // weak-linked and the Swift guards it with `#available(macOS 15.4)`.
    let target = format!("{swift_arch}-apple-macos14.0");
    let lib_path = format!("{out_dir}/libPortBayImagePlayground.a");

    let mut args: Vec<&str> = vec![
        "-emit-library",
        "-static",
        "-O",
        "-module-name",
        "PortBayImagePlayground",
        "-target",
        target.as_str(),
    ];
    if has_image_playground {
        args.push("-framework");
        args.push("ImagePlayground");
    }
    args.push("-framework");
    args.push("AppKit");
    args.push("-o");
    args.push(lib_path.as_str());
    args.push(src);

    let status = Command::new("swiftc").args(&args).status().expect(
        "swiftc not found — Xcode command line tools are required to build PortBay on macOS",
    );
    assert!(
        status.success(),
        "swiftc failed to build the Image Playground bridge"
    );

    if !has_image_playground {
        println!(
            "cargo:warning=ImagePlayground SDK module not found; built a stub bridge \
             (image generation reports requires_macos_15_4 at runtime). Build with the \
             macOS 15.4+ SDK to enable it."
        );
    }

    println!("cargo:rustc-link-search=native={out_dir}");
    println!("cargo:rustc-link-lib=static=PortBayImagePlayground");

    // The static archive embeds autolink hints for the Swift runtime; point the
    // linker at the toolchain's Swift libs so they resolve (the dylibs ship in
    // the OS at /usr/lib/swift at runtime).
    if let Ok(out) = Command::new("xcrun").args(["-f", "swiftc"]).output() {
        if let Ok(path) = String::from_utf8(out.stdout) {
            let swiftc = PathBuf::from(path.trim());
            if let Some(swift_lib) = swiftc
                .parent()
                .and_then(|b| b.parent())
                .map(|root| root.join("lib/swift/macosx"))
            {
                println!("cargo:rustc-link-search=native={}", swift_lib.display());
            }
        }
    }

    // Unlike the other Swift runtime dylibs (linked via absolute /usr/lib/swift
    // paths), libswift_Concurrency.dylib carries an @rpath install name, so the
    // linker records `@rpath/libswift_Concurrency.dylib` with no way to resolve
    // it — dyld then fails at launch with "no LC_RPATH's found". Add an rpath to
    // /usr/lib/swift, where the OS ships the dylib (in the shared cache), so the
    // reference resolves. Floor is macOS 14, well above Concurrency's OS debut.
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");

    if has_image_playground {
        println!("cargo:rustc-link-lib=framework=ImagePlayground");
    }
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=Foundation");
}

/// Does the active SDK ship the `ImagePlayground` module? It first appears in
/// the macOS 15.4 SDK; older toolchains (including the public-CI runners) lack
/// it. Probed with a throwaway `swiftc -typecheck` of a `canImport` guard so
/// the answer matches exactly what `#if canImport(ImagePlayground)` sees in the
/// bridge source.
#[cfg(target_os = "macos")]
fn image_playground_sdk_available() -> bool {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = match Command::new("swiftc")
        .args(["-typecheck", "-parse-as-library", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        // No swiftc here at all — the build will fail loudly later; assume the
        // module is absent rather than emitting framework directives blindly.
        Err(_) => return false,
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin
            .write_all(b"#if canImport(ImagePlayground)\n#else\n#error(\"unavailable\")\n#endif\n");
    }

    child.wait().map(|s| s.success()).unwrap_or(false)
}
