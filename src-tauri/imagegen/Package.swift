// swift-tools-version: 5.10
// portbay-imagegen — local image-generation sidecar (see
// Sources/portbay-imagegen/main.swift).
//
// A SwiftPM package (like portbay-stt) because the engine is a SwiftPM
// dependency:
//   • StableDiffusion (apple/ml-stable-diffusion, MIT) — SD 1.5 / 2.1 / SDXL,
//     Core ML on the Neural Engine / GPU.
//   • Transformers    (huggingface/swift-transformers, Apache-2) — the `Hub`
//     downloader, used to fetch Apple's pre-converted Core ML weights from
//     Hugging Face.
//
// Built by scripts/build-imagegen.sh into
// src-tauri/binaries/portbay-imagegen-<triple>.
//
// Platform floor is macOS 14 (Core ML compute-plan + the engine's minimum). On
// older macOS the exec itself fails; the app maps that to "requires_macos_14"
// and the Image-generation category shows the unavailable state (same degrade
// shape as portbay-stt).
//
// NOTE: Argmax's DiffusionKit was the original engine pick (FLUX/SD3), but its
// Swift/MLX backend is still "🚧" and its README defers to apple/ml-stable-
// diffusion for the Core ML Swift path — so that's the engine here. FLUX/SD3
// land later via the same `engine`-routed protocol once a stable Swift API
// exists, with no app release (the catalog is live-signed).
import PackageDescription

let package = Package(
    name: "portbay-imagegen",
    platforms: [
        .macOS(.v14)
    ],
    dependencies: [
        .package(url: "https://github.com/apple/ml-stable-diffusion.git", from: "1.1.0"),
        .package(url: "https://github.com/huggingface/swift-transformers.git", from: "0.1.8"),
    ],
    targets: [
        .executableTarget(
            name: "portbay-imagegen",
            dependencies: [
                .product(name: "StableDiffusion", package: "ml-stable-diffusion"),
                .product(name: "Transformers", package: "swift-transformers"),
            ],
            path: "Sources/portbay-imagegen"
        )
    ]
)
