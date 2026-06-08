// swift-tools-version: 5.10
// portbay-stt — local speech-to-text sidecar (see Sources/portbay-stt/main.swift).
//
// A SwiftPM package (unlike portbay-afm's single-file swiftc build) because
// the two engine libraries are SwiftPM dependencies:
//   • WhisperKit  (argmaxinc/argmax-oss-swift, MIT)    — Whisper family, CoreML
//   • FluidAudio  (FluidInference/FluidAudio, Apache-2) — Parakeet, CoreML
// Built by scripts/build-stt.sh into src-tauri/binaries/portbay-stt-<triple>.
//
// Platform floor is macOS 14 — FluidAudio's minimum. On older macOS the exec
// itself fails; the app maps that to a "requires_macos_14" status and the
// macOS Dictation engine stays the only transcription option (same degrade
// shape as portbay-afm's requires_macos_26).
import PackageDescription

let package = Package(
    name: "portbay-stt",
    platforms: [
        .macOS(.v14)
    ],
    dependencies: [
        .package(url: "https://github.com/argmaxinc/argmax-oss-swift.git", from: "1.0.0"),
        .package(url: "https://github.com/FluidInference/FluidAudio.git", from: "0.15.1"),
    ],
    targets: [
        .executableTarget(
            name: "portbay-stt",
            dependencies: [
                .product(name: "WhisperKit", package: "argmax-oss-swift"),
                .product(name: "FluidAudio", package: "FluidAudio"),
            ],
            path: "Sources/portbay-stt"
        )
    ]
)
