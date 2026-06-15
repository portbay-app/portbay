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
        // MLXAudio (SwiftPM package name "MLXAudio") — MLX/Metal neural TTS
        // engines (Chatterbox, …) beyond FluidAudio's CoreML Kokoro. No release
        // tags yet, so pinned to a commit for a reproducible build. MIT.
        .package(
            url: "https://github.com/Blaizzy/mlx-audio-swift.git",
            revision: "3f6b0553188a921f635df54b5e20442001037336"),
        // Direct deps so the sidecar can `import MLX` (MLXArray) and
        // `import HuggingFace` (Repo.ID / HubCache for the per-model download).
        // Bounds match what mlx-audio-swift already resolves.
        .package(url: "https://github.com/ml-explore/mlx-swift.git", from: "0.31.0"),
        .package(url: "https://github.com/huggingface/swift-huggingface.git", from: "0.9.0"),
    ],
    targets: [
        .executableTarget(
            name: "portbay-stt",
            dependencies: [
                .product(name: "WhisperKit", package: "argmax-oss-swift"),
                .product(name: "FluidAudio", package: "FluidAudio"),
                .product(name: "MLXAudioTTS", package: "mlx-audio-swift"),
                .product(name: "MLXAudioCore", package: "mlx-audio-swift"),
                .product(name: "MLX", package: "mlx-swift"),
                .product(name: "HuggingFace", package: "swift-huggingface"),
            ],
            path: "Sources/portbay-stt"
        )
    ]
)
