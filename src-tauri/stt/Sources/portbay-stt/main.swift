// portbay-stt — local speech-to-text bridge for Smart Dictation.
//
// Why a sidecar: PortBay's local transcription engines are Swift-only —
// WhisperKit (Whisper family) and FluidAudio (Parakeet) both run CoreML
// models on the Neural Engine through Swift APIs the Rust app cannot reach.
// This CLI is the bridge, modeled on portbay-afm: line-delimited JSON over
// stdin/stdout, spawned and owned by src-tauri/src/stt.rs.
//
// Unlike portbay-afm's strictly serial loop, capture streams events while
// the loop keeps reading stdin (stop-capture must land mid-capture), so all
// stdout writes go through one serialized writer.
//
// Protocol:
//   portbay-stt --check
//     → stdout: one-line JSON {"available":Bool,"reason":String?,
//       "engines":[String]?}; exit 0. The binary only launches on macOS 14+
//       (deployment target) — the app maps a failed exec to requires_macos_14.
//   portbay-stt --serve
//     ← stdin:  one JSON request per line: {"op":String, ...op fields}.
//     → stdout: zero or more event lines {"event":String, ...} followed by
//       exactly one terminal response line {"op":String,"ok":Bool, ...} per
//       request. EOF on stdin = shutdown (exit 0).
//
//   Ops (modelsDir is passed on every request — the app owns the pref):
//     catalog                          → {"op":"catalog","ok":true,"models":[CatalogModel]}
//     installed{modelsDir}             → {"op":"installed","ok":true,"models":[InstalledModel]}
//     download{modelsDir,model,downloadId}
//       events {"event":"progress","downloadId":String,"fraction":Double,
//               "phase":"downloading"|"compiling"}
//                                      → {"op":"download","ok":Bool,"downloadId":String,...}
//       Runs concurrently — the loop keeps reading so cancel-download can
//       land mid-download. Cancelled downloads answer code 6; partial files
//       stay on disk (the hub downloader resumes them) but never count as
//       installed (a `.portbay-complete` marker seals a finished install).
//     cancel-download{downloadId}      → {"op":"cancel-download","ok":true}
//     delete{modelsDir,model}          → {"op":"delete","ok":Bool,...}
//     prewarm{modelsDir,model}         → {"op":"prewarm","ok":Bool,...}
//     start-capture{modelsDir,model}   (engine comes from the catalog entry)
//       events {"event":"listening"} · {"event":"partial","text":String}
//              {"event":"level","rms":Double}
//                                      → no terminal line on success — the
//                                        `listening` event IS the
//                                        confirmation; a start failure does
//                                        answer {"ok":false,...}.
//     stop-capture
//       events {"event":"final","text":String} · {"event":"ended"}
//                                      → {"op":"stop-capture","ok":true}
//     cancel-capture
//       events {"event":"ended"}       → {"op":"cancel-capture","ok":true}
//
//   Error responses: {"op":String,"ok":false,"code":Int,"error":String} —
//   codes mirror portbay-afm: 2 model/engine unavailable · 4 bad request ·
//   5 operation failed · 6 cancelled by request.
//
// Built by scripts/build-stt.sh (SwiftPM release build, deployment target
// macOS 14 — FluidAudio's floor).

import AVFoundation
import CoreML
import CryptoKit
import Foundation
import FluidAudio
import WhisperKit

// MARK: - Wire types

/// One request line. A single struct with optional fields (not an enum with
/// custom decoding) — `op` dispatches, handlers validate the fields they need.
struct Request: Decodable {
    let op: String
    let modelsDir: String?
    let model: String?
    let downloadId: String?
    /// Recognizer bias terms for `start-capture` (workspace/profile/jargon
    /// vocabulary, resolved + capped app-side). WhisperKit tokenizes them into
    /// `DecodingOptions.promptTokens`; engines without a text-prompt seam ignore
    /// them. Absent / empty = no bias.
    let biasTerms: [String]?
    /// Optional model spec carried by the request (download/prewarm/capture).
    /// When present, the sidecar uses these instead of looking the id up in its
    /// bundled `CATALOG` — this is what lets the live PortBay Model Catalog
    /// (Rust-owned) ship new same-engine models with NO sidecar rebuild. Absent
    /// = fall back to `CATALOG`, then to the spec persisted at download time.
    let engine: String?
    let repoModel: String?
    /// Parakeet model generation ("v2" | "v3"); ignored by other engines.
    let parakeetVersion: String?
    let approxSizeBytes: Int64?
    /// Expected install-content digest from the signed catalog (see
    /// `directoryContentDigest`). When present, the download is verified
    /// against it before the install is sealed; absent = no verification
    /// (entries adopt digests incrementally as the catalog publishes them).
    let contentDigest: String?
    /// Text-to-speech: the text to synthesize and the voice id (`tts-synthesize`).
    let text: String?
    let voice: String?
}

/// Terminal response for a request. Encoded as one stdout line.
struct Response: Encodable {
    let op: String
    let ok: Bool
    var code: Int? = nil
    var error: String? = nil
    var models: [CatalogModel]? = nil
    var installed: [InstalledModel]? = nil
    /// Echoed on download terminals so the app can match a response to its
    /// (concurrent) download when several run back-to-back.
    var downloadId: String? = nil
    /// `tts-synthesize` result: base64 of a 24 kHz mono 16-bit PCM WAV.
    var wavBase64: String? = nil
}

/// Sidecar-initiated event line (capture stream, download progress).
/// Progress is fraction-based — both engine libraries report fractions, not
/// bytes (the app estimates bytes from the catalog's approximate size).
struct Event: Encodable {
    let event: String
    var downloadId: String? = nil
    var fraction: Double? = nil
    var phase: String? = nil
    var text: String? = nil
    var rms: Double? = nil
}

// MARK: - Catalog

/// One curated catalog entry. `id` is PortBay's stable identifier (pref
/// value, install dir name); `repoModel` is the engine library's variant
/// name. Sizes are approximate (display only) — the real on-disk size is
/// measured after install.
struct CatalogModel: Encodable {
    let id: String
    /// "whisper" (WhisperKit) or "parakeet" (FluidAudio) — selects the
    /// engine library for download and capture.
    let engine: String
    let displayName: String
    let repoModel: String
    let approxSizeBytes: Int64
    /// Human summary, e.g. "English" or "25 European languages".
    let languages: String
    /// One-line speed/accuracy positioning for the catalog card.
    let speedNote: String
    let recommended: Bool
    /// Whether capture emits live partial transcripts. Batch models show a
    /// clock + "transcribing…" instead.
    let streaming: Bool
    /// Parakeet generation ("v2" | "v3") — selects the FluidAudio download
    /// version. nil for Whisper (and defaults to v3 for Parakeet if unset).
    var parakeetVersion: String? = nil
    /// Expected install-content digest (live catalog only; the bundled
    /// CATALOG carries none). Verified before the install is sealed.
    var contentDigest: String? = nil
}

/// Curated, static — STT models don't live in any registry PortBay could
/// query (Ollama's library has zero STT models), so PortBay ships its own
/// shortlist. Order is display order. Sizes verified against the HF repos
/// when download lands (Phase 2); until then they are honest approximations.
let CATALOG: [CatalogModel] = [
    CatalogModel(
        id: "parakeet-eou-streaming",
        engine: "parakeet-eou",
        displayName: "Parakeet EOU 120M (streaming)",
        // StreamingModelVariant raw value — selects the 320 ms chunk tier
        // (the balanced export: lower WER than batch TDT on short-form, with
        // chunk-level latency). Other tiers can ship via the live catalog.
        repoModel: "parakeet-eou-320ms",
        approxSizeBytes: 300_000_000,
        languages: "English",
        speedNote: "True streaming — words decode while you speak, so the text is ready the instant you stop.",
        recommended: true,
        streaming: true
    ),
    CatalogModel(
        id: "parakeet-tdt-v3",
        engine: "parakeet",
        displayName: "Parakeet TDT v3 (0.6B)",
        repoModel: "parakeet-tdt-0.6b-v3-coreml",
        approxSizeBytes: 2_400_000_000,
        languages: "25 European languages",
        speedNote: "Fastest on Apple Silicon — near-instant transcription on the Neural Engine.",
        recommended: true,
        streaming: false,
        parakeetVersion: "v3"
    ),
    CatalogModel(
        id: "parakeet-tdt-v2",
        engine: "parakeet",
        displayName: "Parakeet TDT v2 (0.6B, English)",
        repoModel: "parakeet-tdt-0.6b-v2-coreml",
        approxSizeBytes: 2_400_000_000,
        languages: "English",
        speedNote: "English-only Parakeet — highest accuracy on English at the same near-instant speed.",
        recommended: false,
        streaming: false,
        parakeetVersion: "v2"
    ),
    CatalogModel(
        id: "whisper-large-v3-turbo",
        engine: "whisper",
        displayName: "Whisper Large v3 Turbo",
        repoModel: "large-v3-v20240930",
        approxSizeBytes: 1_600_000_000,
        languages: "Multilingual (99 languages)",
        speedNote: "Best accuracy-per-second in the Whisper family — the default Whisper pick.",
        recommended: true,
        streaming: true
    ),
    CatalogModel(
        id: "whisper-distil-large-v3",
        engine: "whisper",
        displayName: "Distil-Whisper Large v3",
        repoModel: "distil-large-v3",
        approxSizeBytes: 1_500_000_000,
        languages: "English",
        speedNote: "Distilled large-v3 — close to turbo speed, English only.",
        recommended: false,
        streaming: true
    ),
    CatalogModel(
        id: "whisper-large-v3",
        engine: "whisper",
        displayName: "Whisper Large v3",
        repoModel: "large-v3",
        approxSizeBytes: 3_100_000_000,
        languages: "Multilingual (99 languages)",
        speedNote: "Most accurate, slowest — for when every word matters more than latency.",
        recommended: false,
        streaming: true
    ),
    CatalogModel(
        id: "whisper-medium-en",
        engine: "whisper",
        displayName: "Whisper Medium (English)",
        repoModel: "medium.en",
        approxSizeBytes: 1_500_000_000,
        languages: "English",
        speedNote: "Mid-size English-only — a lighter download for English dictation.",
        recommended: false,
        streaming: true
    ),
    CatalogModel(
        id: "whisper-small",
        engine: "whisper",
        displayName: "Whisper Small",
        repoModel: "small",
        approxSizeBytes: 466_000_000,
        languages: "Multilingual (99 languages)",
        speedNote: "Light multilingual download — quicker and smaller than the large models.",
        recommended: false,
        streaming: true
    ),
    CatalogModel(
        id: "whisper-base",
        engine: "whisper",
        displayName: "Whisper Base",
        repoModel: "base",
        approxSizeBytes: 142_000_000,
        languages: "Multilingual (99 languages)",
        speedNote: "Tiny footprint, fast — good for quick notes where accuracy is less critical.",
        recommended: false,
        streaming: true
    ),
    CatalogModel(
        id: "whisper-tiny",
        engine: "whisper",
        displayName: "Whisper Tiny",
        repoModel: "tiny",
        approxSizeBytes: 75_000_000,
        languages: "Multilingual (99 languages)",
        speedNote: "Smallest, fastest Whisper — lowest accuracy; for constrained machines.",
        recommended: false,
        streaming: true
    ),
]

// MARK: - Installed models

struct InstalledModel: Encodable {
    let id: String
    let engine: String
    let sizeBytes: Int64
}

/// Marker file sealing a finished install. Written only after the engine
/// library's download returns — its absence means "partial / interrupted",
/// which must never count as installed (a half-downloaded CoreML model
/// fails at load with a much worse error than "not installed").
let COMPLETE_MARKER = ".portbay-complete"

/// Written next to [`COMPLETE_MARKER`] at download time so capture, prewarm,
/// and installed-detection work for models that aren't in the bundled
/// [`CATALOG`] — i.e. models the live PortBay Model Catalog added without a
/// sidecar rebuild. The download op already knows the engine/repoModel/version
/// (from the request or `CATALOG`); persisting them removes the only reason the
/// sidecar still needed a static catalog.
let SPEC_MARKER = ".portbay-spec.json"

struct ModelSpec: Codable {
    let engine: String
    let repoModel: String
    var parakeetVersion: String? = nil
}

/// Per-model install root: `<modelsDir>/<catalog id>/`. Each engine library
/// is pointed inside this folder, so delete is one directory removal.
func modelRoot(_ modelsDir: String, _ id: String) -> URL {
    URL(fileURLWithPath: modelsDir).appendingPathComponent(id, isDirectory: true)
}

func writeSpec(_ root: URL, _ entry: CatalogModel) {
    let spec = ModelSpec(
        engine: entry.engine, repoModel: entry.repoModel, parakeetVersion: entry.parakeetVersion)
    if let data = try? JSONEncoder().encode(spec) {
        try? data.write(to: root.appendingPathComponent(SPEC_MARKER))
    }
}

func readSpec(_ root: URL) -> ModelSpec? {
    guard let data = try? Data(contentsOf: root.appendingPathComponent(SPEC_MARKER)) else {
        return nil
    }
    return try? JSONDecoder().decode(ModelSpec.self, from: data)
}

// MARK: - Install-content digest

enum DigestError: Error, CustomStringConvertible {
    case mismatch(expected: String, actual: String)
    case unreadable(String)
    var description: String {
        switch self {
        case .mismatch(let expected, let actual):
            return "downloaded model failed integrity verification "
                + "(expected \(expected.prefix(12))…, got \(actual.prefix(12))…) — "
                + "delete and re-download, or update PortBay"
        case .unreadable(let path):
            return "could not read \(path) while verifying the download"
        }
    }
}

/// Streaming SHA-256 of one file (weights run to multiple GB — never load
/// them whole).
func fileSHA256(_ url: URL) throws -> String {
    guard let handle = try? FileHandle(forReadingFrom: url) else {
        throw DigestError.unreadable(url.lastPathComponent)
    }
    defer { try? handle.close() }
    var hasher = SHA256()
    while true {
        let chunk = autoreleasepool { handle.readData(ofLength: 8 << 20) }
        if chunk.isEmpty { break }
        hasher.update(data: chunk)
    }
    return hasher.finalize().map { String(format: "%02x", $0) }.joined()
}

/// Canonical content digest of an install root: SHA-256 over the sorted
/// `relative-path:sha256` lines of every regular file (PortBay's own
/// `.portbay-*` markers excluded). Engine downloads are multi-file HF
/// snapshots with no single artifact to pin, so the catalog pins this
/// directory digest instead — the same role the sha256-pinned runtimes
/// manifest plays for binaries. Deterministic: path-sorted, content-only
/// (no mtimes/permissions).
func directoryContentDigest(_ root: URL) throws -> String {
    let fm = FileManager.default
    var lines: [String] = []
    guard
        let it = fm.enumerator(
            at: root, includingPropertiesForKeys: [.isRegularFileKey],
            options: [.skipsHiddenFiles])
    else {
        throw DigestError.unreadable(root.path)
    }
    let prefix = root.standardizedFileURL.path + "/"
    for case let url as URL in it {
        guard (try? url.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true
        else { continue }
        let rel = String(url.standardizedFileURL.path.dropFirst(prefix.count))
        if rel == COMPLETE_MARKER || rel == SPEC_MARKER { continue }
        lines.append("\(rel):\(try fileSHA256(url))")
    }
    lines.sort()
    let manifest = Data(lines.joined(separator: "\n").utf8)
    return SHA256.hash(data: manifest).map { String(format: "%02x", $0) }.joined()
}

/// Verify a finished download against the catalog's expected digest, when one
/// was provided. Throwing here means the install is NOT sealed (no
/// `.portbay-complete`), so a tampered or corrupted download can never load.
func verifyContentDigest(_ root: URL, _ entry: CatalogModel) throws {
    guard let expected = entry.contentDigest?.lowercased(), !expected.isEmpty else { return }
    let actual = try directoryContentDigest(root)
    if actual != expected {
        throw DigestError.mismatch(expected: expected, actual: actual)
    }
}

/// Resolve the engine/repoModel/version for an op. Priority: spec carried by
/// the request (live catalog) → the bundled `CATALOG` → the spec persisted on
/// disk at download time. nil means "not enough to act".
func resolveEntry(modelsDir: String?, request: Request) -> CatalogModel? {
    guard let id = request.model, !id.isEmpty else { return nil }
    if let engine = request.engine, !engine.isEmpty,
        let repo = request.repoModel, !repo.isEmpty
    {
        return CatalogModel(
            id: id, engine: engine, displayName: id, repoModel: repo,
            approxSizeBytes: request.approxSizeBytes ?? 0, languages: "", speedNote: "",
            recommended: false, streaming: streamingEngine(engine),
            parakeetVersion: request.parakeetVersion,
            contentDigest: request.contentDigest)
    }
    if let entry = CATALOG.first(where: { $0.id == id }) { return entry }
    if let dir = modelsDir, !dir.isEmpty, let spec = readSpec(modelRoot(dir, id)) {
        return CatalogModel(
            id: id, engine: spec.engine, displayName: id, repoModel: spec.repoModel,
            approxSizeBytes: 0, languages: "", speedNote: "", recommended: false,
            streaming: streamingEngine(spec.engine), parakeetVersion: spec.parakeetVersion)
    }
    return nil
}

/// Whether an engine streams live partials during capture. Whisper streams by
/// re-transcription; the EOU/Nemotron families stream natively (cache-aware
/// chunk decode). Batch engines (Parakeet TDT, Qwen3, Cohere) don't.
func streamingEngine(_ engine: String) -> Bool {
    engine == "whisper" || engine == "parakeet-eou" || engine == "nemotron"
}

/// A model is installed when its root exists AND carries the completion
/// marker. Scans the models dir from disk (not `CATALOG`) so live-catalog
/// models show as installed too; engine comes from the persisted spec, falling
/// back to the bundled catalog for pre-spec installs.
func scanInstalled(modelsDir: String) -> [InstalledModel] {
    let fm = FileManager.default
    guard let ids = try? fm.contentsOfDirectory(atPath: modelsDir) else { return [] }
    return ids.compactMap { id in
        let root = modelRoot(modelsDir, id)
        guard fm.fileExists(atPath: root.appendingPathComponent(COMPLETE_MARKER).path) else {
            return nil
        }
        let engine = readSpec(root)?.engine
            ?? CATALOG.first(where: { $0.id == id })?.engine
            ?? "whisper"
        return InstalledModel(id: id, engine: engine, sizeBytes: directorySize(root.path))
    }
}

func directorySize(_ path: String) -> Int64 {
    let fm = FileManager.default
    guard let walker = fm.enumerator(atPath: path) else { return 0 }
    var total: Int64 = 0
    while let rel = walker.nextObject() as? String {
        let full = (path as NSString).appendingPathComponent(rel)
        if let attrs = try? fm.attributesOfItem(atPath: full),
            let size = attrs[.size] as? Int64,
            (attrs[.type] as? FileAttributeType) == .typeRegular
        {
            total += size
        }
    }
    return total
}

// MARK: - Serialized stdout writer

/// All stdout writes funnel through here: capture events arrive from a
/// background task while the serve loop answers other requests, and two
/// interleaved half-lines would corrupt the line-delimited protocol.
/// FileHandle writes are unbuffered syscalls — the lock is the only thing
/// needed for atomic lines.
final class LineWriter: @unchecked Sendable {
    static let shared = LineWriter()
    private let lock = NSLock()

    private func writeLine(_ data: Data) {
        lock.lock()
        defer { lock.unlock() }
        FileHandle.standardOutput.write(data)
        FileHandle.standardOutput.write(Data("\n".utf8))
    }

    func send(_ response: Response) {
        let data = (try? JSONEncoder().encode(response))
            ?? Data("{\"op\":\"\(response.op)\",\"ok\":false,\"code\":5,\"error\":\"response encode failed\"}".utf8)
        writeLine(data)
    }

    func send(_ event: Event) {
        guard let data = try? JSONEncoder().encode(event) else { return }
        writeLine(data)
    }
}

// MARK: - Downloads

/// In-flight downloads, keyed by the app-chosen downloadId. An actor because
/// the serve loop registers/cancels while download tasks self-remove on
/// completion.
actor ActiveDownloads {
    static let shared = ActiveDownloads()
    private var tasks: [String: Task<Void, Never>] = [:]

    func register(_ id: String, _ task: Task<Void, Never>) {
        tasks[id] = task
    }

    func cancel(_ id: String) {
        tasks.removeValue(forKey: id)?.cancel()
    }

    func finished(_ id: String) {
        tasks.removeValue(forKey: id)
    }
}

/// Progress relay with rate limiting: the hub downloaders fire per-chunk —
/// unthrottled that's thousands of stdout lines per model. Emit on ≥0.5%
/// movement only; the terminal response, not a 1.0 event, signals "done".
final class ProgressGate: @unchecked Sendable {
    private let lock = NSLock()
    private var lastSent: Double = -1

    func relay(_ downloadId: String, _ fraction: Double, phase: String) {
        lock.lock()
        let due = fraction - lastSent >= 0.005
        if due { lastSent = fraction }
        lock.unlock()
        guard due else { return }
        LineWriter.shared.send(
            Event(event: "progress", downloadId: downloadId, fraction: min(max(fraction, 0), 1), phase: phase))
    }
}

/// Run one model download to its install root. Blocking inside a registered
/// Task — cancellation arrives as cooperative Task cancellation, which both
/// engines' URLSession-backed downloaders honor.
func runDownload(modelsDir: String, entry: CatalogModel, downloadId: String) async {
    let out = LineWriter.shared
    let root = modelRoot(modelsDir, entry.id)
    let gate = ProgressGate()
    do {
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        switch entry.engine {
        case "whisper":
            // Hub snapshot layout lands the model under
            // <root>/models/argmaxinc/whisperkit-coreml/<variant folder>;
            // load (capture phase) re-finds it by scanning for *.mlmodelc.
            // Progress is scaled into [0, 0.9] — the load below is the rest.
            _ = try await WhisperKit.download(
                variant: entry.repoModel,
                downloadBase: root,
                progressCallback: { progress in
                    gate.relay(downloadId, progress.fractionCompleted * 0.9, phase: "downloading")
                }
            )
            try Task.checkCancellation()
            // Load once before sealing: fetches the tokenizer (which
            // WhisperKit otherwise pulls at FIRST USE — an offline first
            // dictation would fail) into the model root, and triggers
            // CoreML's on-device specialization so the first capture
            // doesn't pay the compile.
            gate.relay(downloadId, 0.92, phase: "compiling")
            _ = try await loadEngine(modelsDir: modelsDir, entry: entry, allowUnsealed: true)
        case "parakeet":
            // AsrModels resolves everything against the parent of the dir it
            // is given (repoPath) — pass <root>/<repo folder> so files land
            // inside the model root and load() gets the identical path.
            let target = root.appendingPathComponent(entry.repoModel, isDirectory: true)
            let version: AsrModelVersion = entry.parakeetVersion == "v2" ? .v2 : .v3
            _ = try await AsrModels.download(
                to: target,
                version: version,
                progressHandler: { progress in
                    let phase: String
                    switch progress.phase {
                    case .compiling: phase = "compiling"
                    default: phase = "downloading"
                    }
                    gate.relay(downloadId, progress.fractionCompleted, phase: phase)
                }
            )
        default:
            out.send(
                Response(
                    op: "download", ok: false, code: 4,
                    error: "unknown engine: \(entry.engine)", downloadId: downloadId))
            await ActiveDownloads.shared.finished(downloadId)
            return
        }
        try Task.checkCancellation()
        // Verify against the catalog's expected digest (no-op when absent)
        // BEFORE sealing — a corrupted/tampered download must never load.
        try verifyContentDigest(root, entry)
        // Persist the spec so capture/prewarm/installed work for this model
        // even when it isn't in the bundled CATALOG (live-catalog models).
        writeSpec(root, entry)
        // Seal the install — only now does the model count as installed.
        FileManager.default.createFile(atPath: root.appendingPathComponent(COMPLETE_MARKER).path, contents: nil)
        out.send(Response(op: "download", ok: true, downloadId: downloadId))
    } catch {
        // Both engines wrap the CancellationError (observed: WhisperKit
        // rethrows it as "Download failed: cancelled"), so cancelled-ness is
        // detected from the task, not the error type. Partial files stay for
        // resume; no marker = not installed.
        if Task.isCancelled || error is CancellationError {
            out.send(Response(op: "download", ok: false, code: 6, error: "cancelled", downloadId: downloadId))
        } else {
            out.send(
                Response(
                    op: "download", ok: false, code: 5,
                    error: "download failed: \(error.localizedDescription)", downloadId: downloadId))
        }
    }
    await ActiveDownloads.shared.finished(downloadId)
}

/// Download a Qwen3 / Cohere / Nemotron model. Each uses its own FluidAudio
/// downloader; files land under the install root and load locates them by
/// scanning for `.mlmodelc` (see `loadEngine`).
func runAdvancedDownload(modelsDir: String, entry: CatalogModel, downloadId: String) async {
    let out = LineWriter.shared
    let root = modelRoot(modelsDir, entry.id)
    let gate = ProgressGate()
    let progress: DownloadUtils.ProgressHandler = { p in
        gate.relay(downloadId, p.fractionCompleted, phase: "downloading")
    }
    do {
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        switch entry.engine {
        case "qwen3":
            guard #available(macOS 15, *) else {
                throw SttError.badEngine("Qwen3-ASR requires macOS 15 or newer")
            }
            _ = try await Qwen3AsrModels.download(variant: .f32, to: root, progressHandler: progress)
        case "cohere":
            try await DownloadUtils.downloadRepo(.cohereTranscribeCoreml, to: root, progressHandler: progress)
        case "parakeet-eou":
            // Tier selected by repoModel (StreamingModelVariant raw value) so
            // load gets the matching chunk-size export. 320 ms is the curated
            // default (better WER than batch TDT at a fraction of the size).
            let variant = StreamingModelVariant(rawValue: entry.repoModel) ?? .parakeetEou320ms
            try await DownloadUtils.downloadRepo(variant.repo, to: root, progressHandler: progress)
        case "nemotron":
            // Honor a tier carried as a StreamingModelVariant raw value; the
            // original catalog entry carries the HF repo name instead, which
            // keeps the historical 1120 ms tier.
            let repo = StreamingModelVariant(rawValue: entry.repoModel)?.repo ?? .nemotronStreaming1120
            try await DownloadUtils.downloadRepo(repo, to: root, progressHandler: progress)
        default:
            throw SttError.badEngine(entry.engine)
        }
        try Task.checkCancellation()
        try verifyContentDigest(root, entry)
        writeSpec(root, entry)
        FileManager.default.createFile(
            atPath: root.appendingPathComponent(COMPLETE_MARKER).path, contents: nil)
        out.send(Response(op: "download", ok: true, downloadId: downloadId))
    } catch {
        if Task.isCancelled || error is CancellationError {
            out.send(
                Response(op: "download", ok: false, code: 6, error: "cancelled", downloadId: downloadId))
        } else {
            out.send(
                Response(
                    op: "download", ok: false, code: 5,
                    error: "download failed: \(error.localizedDescription)", downloadId: downloadId))
        }
    }
    await ActiveDownloads.shared.finished(downloadId)
}

// MARK: - Text-to-Speech (Kokoro via FluidAudio)

/// The 28 English Kokoro voices (af/am = American, bf/bm = British). FluidAudio's
/// English ANE bundle ships ONLY `af_heart`; the rest are byte-identical
/// `[510,256]` fp32 voice packs from the upstream Kokoro repo. We stage them into
/// the model root so FluidAudio's on-demand `ensureVoicePack` finds them locally
/// instead of 404ing against the CoreML repo (which has only af_heart).
let KOKORO_EN_VOICES: [String] = [
    "af_heart", "af_alloy", "af_aoede", "af_bella", "af_jessica", "af_kore",
    "af_nicole", "af_nova", "af_river", "af_sarah", "af_sky",
    "am_adam", "am_echo", "am_eric", "am_fenrir", "am_liam", "am_michael",
    "am_onyx", "am_puck", "am_santa",
    "bf_alice", "bf_emma", "bf_isabella", "bf_lily",
    "bm_daniel", "bm_fable", "bm_george", "bm_lewis",
]

/// Pinned to a commit (not `main`) so the fetched bytes are reproducible. These
/// are voice-embedding tensors, not code.
let KOKORO_VOICE_BASE_URL =
    "https://raw.githubusercontent.com/hexgrad/kokoro/dfb907a02bba8152ca444717ca5d78747ccb4bec/kokoro.js/voices"

/// `[510, 256]` fp32 — the exact size FluidAudio's voice-pack loader expects.
/// Used to reject a 404 HTML body or a truncated transfer.
let KOKORO_VOICE_PACK_BYTES = 510 * 256 * 4

enum KokoroVoiceError: Error { case badDownload(String, Int) }

/// Stage one English Kokoro voice pack into `root` (idempotent). No-op when the
/// file is already present at the expected size.
func fetchKokoroVoiceIfNeeded(_ voice: String, into root: URL) async throws {
    let dest = root.appendingPathComponent("\(voice).bin")
    if let attrs = try? FileManager.default.attributesOfItem(atPath: dest.path),
        (attrs[.size] as? Int) == KOKORO_VOICE_PACK_BYTES
    {
        return
    }
    guard KOKORO_EN_VOICES.contains(voice),
        let url = URL(string: "\(KOKORO_VOICE_BASE_URL)/\(voice).bin")
    else { return }
    let (data, response) = try await URLSession(configuration: .ephemeral).data(from: url)
    guard let http = response as? HTTPURLResponse, http.statusCode == 200,
        data.count == KOKORO_VOICE_PACK_BYTES
    else {
        throw KokoroVoiceError.badDownload(voice, data.count)
    }
    try data.write(to: dest, options: [.atomic])
}

/// Stage all English Kokoro voice packs into `repoDir` (the dir `ensureModels`
/// returns, where FluidAudio's flat English layout looks for `<voice>.bin`),
/// reporting a "voices" phase. Throws on the first failure so a fresh install
/// surfaces it; the synth-time net (`fetchKokoroVoiceIfNeeded`) recovers any
/// that were skipped.
func ensureKokoroEnglishVoices(repoDir: URL, downloadId: String?) async throws {
    let gate = ProgressGate()
    let total = KOKORO_EN_VOICES.count
    for (i, voice) in KOKORO_EN_VOICES.enumerated() {
        try Task.checkCancellation()
        try await fetchKokoroVoiceIfNeeded(voice, into: repoDir)
        if let id = downloadId {
            gate.relay(id, Double(i + 1) / Double(total), phase: "voices")
        }
    }
}

/// Download the Kokoro mlmodelc chain for a TTS catalog entry. Mirrors
/// `runDownload`'s shape (progress events + seal + spec) so the app's STT
/// download UI works unchanged for voices. Voices themselves are small `.bin`
/// packs fetched on demand at first synth.
func runTtsDownload(modelsDir: String, entry: CatalogModel, downloadId: String) async {
    let out = LineWriter.shared
    let root = modelRoot(modelsDir, entry.id)
    let gate = ProgressGate()
    do {
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        // `ensureModels` returns the actual repo dir (root/<repo.folderName>),
        // which is where the English voice packs must live — not `root` itself.
        let repoDir = try await KokoroAneResourceDownloader.ensureModels(
            variant: .english,
            directory: root,
            progressHandler: { progress in
                gate.relay(downloadId, progress.fractionCompleted, phase: "downloading")
            }
        )
        try Task.checkCancellation()
        // Stage the full English voice set (af_heart already arrived above).
        // Best-effort: a transient voice fetch must not fail the multi-hundred-MB
        // model install — the synth-time net re-fetches any that were missed.
        do {
            try await ensureKokoroEnglishVoices(repoDir: repoDir, downloadId: downloadId)
        } catch is CancellationError {
            throw CancellationError()
        } catch {
            FileHandle.standardError.write(
                Data("portbay-stt: some Kokoro voices deferred to first use: \(error)\n".utf8))
        }
        // Verify (no-op without a catalog digest). The digest covers the
        // model snapshot AND the staged voice packs, so a catalog entry
        // carrying one must be computed after the full voice set is staged.
        try verifyContentDigest(root, entry)
        writeSpec(root, entry)
        FileManager.default.createFile(
            atPath: root.appendingPathComponent(COMPLETE_MARKER).path, contents: nil)
        out.send(Response(op: "download", ok: true, downloadId: downloadId))
    } catch {
        if Task.isCancelled || error is CancellationError {
            out.send(
                Response(op: "download", ok: false, code: 6, error: "cancelled", downloadId: downloadId))
        } else {
            out.send(
                Response(
                    op: "download", ok: false, code: 5,
                    error: "synthesis model download failed: \(error.localizedDescription)",
                    downloadId: downloadId))
        }
    }
    await ActiveDownloads.shared.finished(downloadId)
}

/// One resident Kokoro synthesizer, reused across synths (the 7 mlmodelcs stay
/// loaded). Re-keys when the model dir changes.
actor TtsCache {
    static let shared = TtsCache()
    private var manager: KokoroAneManager?
    private var key: String?

    func synthesizer(modelsDir: String, id: String) async throws -> KokoroAneManager {
        let root = modelRoot(modelsDir, id)
        if let m = manager, key == root.path {
            return m
        }
        let m = KokoroAneManager(variant: .english, directory: root)
        try await m.initialize()
        manager = m
        key = root.path
        return m
    }
}

/// Synthesize `text` to a base64 WAV (24 kHz mono 16-bit PCM). `voice` is a
/// Kokoro voice id (e.g. "af_heart"); nil uses the variant default. The voice
/// pack is fetched on demand by the synthesizer if missing.
func synthesizeTts(modelsDir: String, id: String, text: String, voice: String?) async throws
    -> String
{
    // Safety net for models installed before the full voice set shipped (or any
    // voice the bulk stage skipped): make sure the requested pack is on disk
    // before FluidAudio looks for it — otherwise it 404s against the CoreML repo.
    // `ensureModels` is a cache-hit here (the model is already installed) and
    // returns the same repo dir the synthesizer reads voices from.
    if let v = voice, !v.isEmpty {
        let root = modelRoot(modelsDir, id)
        if let repoDir = try? await KokoroAneResourceDownloader.ensureModels(
            variant: .english, directory: root)
        {
            try? await fetchKokoroVoiceIfNeeded(v, into: repoDir)
        }
    }
    let manager = try await TtsCache.shared.synthesizer(modelsDir: modelsDir, id: id)
    let wav = try await manager.synthesize(text: text, voice: voice)
    return wav.base64EncodedString()
}

// MARK: - Engines

/// A loaded transcription engine. Both run a plain "transcribe these 16 kHz
/// mono floats" call — partials come from re-transcribing the accumulated
/// buffer on a cadence (one code path for both engines) rather than from the
/// libraries' per-engine streaming stacks: Parakeet TDT has no true
/// streaming mode at all, and at CoreML speeds a 2 s re-transcribe cadence
/// is indistinguishable in the partial overlay.
/// Token budget for the recognizer bias prompt. Kept under Whisper's
/// prompt-context ceiling (½·maxTokenContext) so the decoder's own suffix trim
/// never drops our highest-priority terms (the app sends them priority-first).
let BIAS_TOKEN_BUDGET = 100

/// A FluidAudio ASR engine that isn't Whisper or Parakeet — Qwen3, Cohere, or
/// Nemotron. Boxed behind a protocol so [`LoadedEngine`] (which is used on the
/// macOS-14 capture path) can hold one even though some conformers
/// (`Qwen3Engine`) are `@available(macOS 15)`: the existential type is available
/// on 14, only *constructing* an instance is gated (see `loadEngine`).
protocol AdvancedAsrEngine: AnyObject {
    func transcribe(_ samples: [Float]) async throws -> String
}

enum LoadedEngine {
    case whisper(WhisperKit)
    case parakeet(AsrManager)
    /// Qwen3 / Cohere via FluidAudio — batch transcribe, no text-prompt bias
    /// seam, no incremental commit.
    case advanced(AdvancedAsrEngine)
    /// True streaming ASR (Parakeet EOU / Nemotron) — cache-aware encoders
    /// that decode chunk-by-chunk DURING capture, so at stop only the final
    /// partial chunk remains to decode (~tens of ms), independent of
    /// dictation length. The capture session drives these through their own
    /// feed loop (`CaptureSession`'s streaming path), not `transcribe`.
    case streaming(any StreamingAsrManager)

    /// Tokenize recognizer bias terms into Whisper prompt tokens — or `nil` when
    /// the engine can't take a text prompt (Parakeet/TDT has no such seam), the
    /// engine-capability gate in action: we never fake a bias we can't apply.
    /// Whisper reads its prompt as preceding context, so the terms go in as a
    /// plain comma list (a labelled "vocabulary:" header would be read as
    /// content). Special tokens are stripped and the result capped to
    /// [`BIAS_TOKEN_BUDGET`], priority-first.
    func biasPromptTokens(_ terms: [String]) -> [Int]? {
        guard case .whisper(let kit) = self else { return nil }
        let cleaned = terms
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        guard !cleaned.isEmpty, let tokenizer = kit.tokenizer else { return nil }
        let prompt = " " + cleaned.joined(separator: ", ")
        let tokens = tokenizer.encode(text: prompt)
            .filter { $0 < tokenizer.specialTokens.specialTokenBegin }
        guard !tokens.isEmpty else { return nil }
        return Array(tokens.prefix(BIAS_TOKEN_BUDGET))
    }

    /// Transcribe a snapshot of samples. `finalPass` lets Whisper chunk
    /// arbitrarily long audio (VAD); partial passes feed it a single window.
    /// `promptTokens` biases the Whisper decoder toward known vocabulary
    /// (ignored by Parakeet — see `biasPromptTokens`).
    func transcribe(_ samples: [Float], finalPass: Bool, promptTokens: [Int]?) async throws
        -> String
    {
        switch self {
        case .whisper(let kit):
            let options = DecodingOptions(
                task: .transcribe,
                usePrefillPrompt: true,
                // WhisperKit defaults this to false, which leaves
                // `<|startoftranscript|>` / `<|0.00|>` markers in `.text` —
                // they leaked into pastes verbatim. Timestamp tokens are
                // special tokens too, so this cleans the text while the
                // segment timing metadata (incremental-commit boundaries)
                // is unaffected.
                skipSpecialTokens: true,
                promptTokens: promptTokens,
                chunkingStrategy: finalPass ? .vad : nil
            )
            let results = try await kit.transcribe(audioArray: samples, decodeOptions: options)
            let text = results.map(\.text).joined(separator: " ")
                .trimmingCharacters(in: .whitespacesAndNewlines)
            return filterWhisperHallucination(text, segments: results.flatMap(\.segments))
        case .parakeet(let manager):
            // Fresh decoder state per pass — each pass re-reads the whole
            // snapshot, so carrying state would double-decode. (Parakeet's TDT
            // decoder takes no text prompt, so `promptTokens` is unused here.)
            var state = try TdtDecoderState()
            let result = try await manager.transcribe(samples, decoderState: &state)
            return result.text.trimmingCharacters(in: .whitespacesAndNewlines)
        case .advanced(let engine):
            // promptTokens unused — these engines have no text-prompt bias seam.
            let text = try await engine.transcribe(samples)
            return text.trimmingCharacters(in: .whitespacesAndNewlines)
        case .streaming(let manager):
            // Batch adaptation for completeness only — live capture feeds the
            // manager incrementally and never lands here. Reset first so a
            // prior session's decoder state can't leak into this pass.
            try await manager.reset()
            if let buffer = makePCMBuffer(samples) {
                try await manager.appendAudio(buffer)
                try await manager.processBufferedAudio()
            }
            let text = try await manager.finish()
            return text.trimmingCharacters(in: .whitespacesAndNewlines)
        }
    }

    /// Whisper-only: VAD-segment a window and hand back the raw segments (text +
    /// end time + no-speech probability). The incremental commit pass uses the
    /// segment end times to freeze whole words on silence boundaries, and the
    /// no-speech probabilities to reuse the same hallucination guard. Returns an
    /// empty array for Parakeet, which stays on the full-buffer finalize path.
    func whisperSegments(_ samples: [Float], promptTokens: [Int]?) async throws
        -> [TranscriptionSegment]
    {
        guard case .whisper(let kit) = self else { return [] }
        let options = DecodingOptions(
            task: .transcribe,
            usePrefillPrompt: true,
            skipSpecialTokens: true,
            promptTokens: promptTokens,
            chunkingStrategy: .vad
        )
        let results = try await kit.transcribe(audioArray: samples, decodeOptions: options)
        return results.flatMap(\.segments)
    }
}

// MARK: - Advanced FluidAudio engines (Qwen3 / Cohere / Nemotron)

/// Qwen3-ASR — clean batch pipeline (macOS 15+).
@available(macOS 15, *)
final class Qwen3Engine: AdvancedAsrEngine {
    private let manager: Qwen3AsrManager
    init(_ manager: Qwen3AsrManager) { self.manager = manager }
    func transcribe(_ samples: [Float]) async throws -> String {
        try await manager.transcribe(audioSamples: samples, language: nil as String?)
    }
}

/// Cohere Transcribe (cohere-transcribe-03-2026) — encoder/decoder CoreML
/// pipeline, batch transcribe (macOS 14+).
final class CohereEngine: AdvancedAsrEngine {
    private let pipeline: CoherePipeline
    private let models: CoherePipeline.LoadedModels
    init(pipeline: CoherePipeline, models: CoherePipeline.LoadedModels) {
        self.pipeline = pipeline
        self.models = models
    }
    func transcribe(_ samples: [Float]) async throws -> String {
        try await pipeline.transcribe(audio: samples, models: models).text
    }
}

/// Build a 16 kHz mono Float32 `AVAudioPCMBuffer` from raw samples — the
/// streaming managers take buffers, not `[Float]`.
func makePCMBuffer(_ samples: [Float], sampleRate: Double = 16_000) -> AVAudioPCMBuffer? {
    guard !samples.isEmpty,
        let format = AVAudioFormat(
            commonFormat: .pcmFormatFloat32, sampleRate: sampleRate, channels: 1,
            interleaved: false),
        let buffer = AVAudioPCMBuffer(
            pcmFormat: format, frameCapacity: AVAudioFrameCount(samples.count))
    else { return nil }
    buffer.frameLength = AVAudioFrameCount(samples.count)
    if let channel = buffer.floatChannelData {
        samples.withUnsafeBufferPointer { src in
            channel[0].update(from: src.baseAddress!, count: samples.count)
        }
    }
    return buffer
}

/// Whisper hallucinates stock YouTube-caption phrases on silence and breath
/// noise — the training data's signature leaking through. Same guard
/// freeflow ships: drop the result only when the model itself doubted
/// speech was present (noSpeechProb on EVERY segment past the threshold)
/// AND the text is one of the known silence artifacts. A real dictation of
/// "Thank you." carries a low noSpeechProb and passes through untouched.
/// Whisper-only — Parakeet's TDT decoder doesn't produce these.
let HALLUCINATION_NO_SPEECH_THRESHOLD: Float = 0.1
let HALLUCINATION_BLOCKLIST: Set<String> = [
    "thank you",
    "thank you for watching",
    "thank you so much",
    "thank you so much for watching",
    "thanks for watching",
    "please subscribe",
    "like and subscribe",
    "subtitles by the amaraorg community",
    "you",
]

func filterWhisperHallucination(_ text: String, segments: [TranscriptionSegment]) -> String {
    guard !text.isEmpty, !segments.isEmpty else { return text }
    // Normalize the way the blocklist is written: lowercase, letters and
    // spaces only ("Thank you for watching!" → "thank you for watching").
    let normalized = text.lowercased()
        .unicodeScalars
        .filter { CharacterSet.lowercaseLetters.contains($0) || $0 == " " }
        .reduce(into: "") { $0.unicodeScalars.append($1) }
        .split(separator: " ")
        .joined(separator: " ")
    guard HALLUCINATION_BLOCKLIST.contains(normalized) else { return text }
    let allDoubtful = segments.allSatisfy { $0.noSpeechProb >= HALLUCINATION_NO_SPEECH_THRESHOLD }
    return allDoubtful ? "" : text
}

/// Locate the Whisper model folder inside a model root: the hub snapshot
/// lands it at <root>/models/<org>/<repo>/<variant folder>. Identified by
/// containing compiled CoreML bundles rather than by reproducing the hub's
/// path math — the layout is the library's implementation detail.
func findWhisperModelFolder(_ root: URL) -> URL? {
    let fm = FileManager.default
    guard
        let walker = fm.enumerator(
            at: root, includingPropertiesForKeys: [.isDirectoryKey],
            options: [.skipsHiddenFiles])
    else { return nil }
    for case let url as URL in walker {
        if url.pathExtension == "mlmodelc" {
            return url.deletingLastPathComponent()
        }
    }
    return nil
}

/// Locate the directory containing a named marker file anywhere under `root`.
/// The streaming repos nest their tier folder (e.g.
/// `<root>/parakeet-realtime-eou-120m-coreml/320ms/`), and the generic
/// first-`.mlmodelc` scan can land INSIDE a nested bundle (Nemotron keeps its
/// encoder under `encoder/<file>.mlmodelc`), so each streaming engine looks
/// for a file that only exists at its model dir's top level.
func findModelFolder(_ root: URL, containing marker: String) -> URL? {
    let fm = FileManager.default
    guard
        let walker = fm.enumerator(
            at: root, includingPropertiesForKeys: [.isDirectoryKey],
            options: [.skipsHiddenFiles])
    else { return nil }
    for case let url as URL in walker {
        if url.lastPathComponent == marker {
            return url.deletingLastPathComponent()
        }
    }
    return nil
}

/// EOU auto-stop debounce: how long speech must stay absent before the model
/// flags End-of-Utterance. The library default (1280 ms) is tuned for
/// turn-taking; 900 ms keeps hands-free auto-stop snappy without clipping
/// normal mid-sentence pauses.
let EOU_DEBOUNCE_MS = 900

/// An `MLModelConfiguration` pinned to CPU+ANE. The bare default (`.all`)
/// lets CoreML route quantized streaming ops to the GPU, which measures ~10×
/// slower than the ANE path for these encoders (the Nemotron manager applies
/// the same pin internally when given no configuration).
func aneConfiguration() -> MLModelConfiguration {
    let config = MLModelConfiguration()
    config.computeUnits = .cpuAndNeuralEngine
    // The few ops CoreML still routes to the GPU may accumulate in fp16 —
    // measurably faster on quantized kernels, no accuracy impact for ASR
    // (FluidVoice ships the same flag on its streaming engines).
    config.allowLowPrecisionAccumulationOnGPU = true
    return config
}

/// Load a model for capture/prewarm. Only sealed installs load (unless the
/// download op itself is doing its verification load via `allowUnsealed`) —
/// both engine libraries silently fall back to DOWNLOADING a missing model
/// at load time, which would turn a mis-configured capture start into a
/// surprise multi-GB download.
func loadEngine(modelsDir: String, entry: CatalogModel, allowUnsealed: Bool = false) async throws
    -> LoadedEngine
{
    let root = modelRoot(modelsDir, entry.id)
    let sealed = FileManager.default.fileExists(
        atPath: root.appendingPathComponent(COMPLETE_MARKER).path)
    guard sealed || allowUnsealed else { throw SttError.notInstalled }
    switch entry.engine {
    case "whisper":
        guard let folder = findWhisperModelFolder(root) else {
            throw SttError.notInstalled
        }
        let config = WhisperKitConfig(
            model: entry.repoModel,
            // Tokenizer was cached under the model root at download time;
            // download stays enabled as the self-repair path for a wiped
            // tokenizer cache (model weights are present, so nothing big
            // can re-download).
            modelFolder: folder.path,
            tokenizerFolder: root,
            load: true
        )
        return .whisper(try await WhisperKit(config))
    case "parakeet":
        let target = root.appendingPathComponent(entry.repoModel, isDirectory: true)
        let version: AsrModelVersion = entry.parakeetVersion == "v2" ? .v2 : .v3
        let models = try await AsrModels.load(from: target, version: version)
        let manager = AsrManager(config: .default)
        try await manager.loadModels(models)
        return .parakeet(manager)
    case "qwen3":
        guard #available(macOS 15, *) else {
            throw SttError.badEngine("Qwen3-ASR requires macOS 15 or newer")
        }
        let dir = findWhisperModelFolder(root) ?? root
        let manager = Qwen3AsrManager()
        try await manager.loadModels(from: dir)
        return .advanced(Qwen3Engine(manager))
    case "cohere":
        let dir = findWhisperModelFolder(root) ?? root
        let pipeline = CoherePipeline()
        let models = try await CoherePipeline.loadModels(
            encoderDir: dir, decoderDir: dir, vocabDir: dir)
        return .advanced(CohereEngine(pipeline: pipeline, models: models))
    case "parakeet-eou":
        // `repoModel` carries the StreamingModelVariant raw value (e.g.
        // "parakeet-eou-320ms") so the live catalog can ship other chunk
        // tiers without a sidecar rebuild. The chunk size MUST match the
        // downloaded tier — each tier is a separately exported encoder.
        let variant = StreamingModelVariant(rawValue: entry.repoModel) ?? .parakeetEou320ms
        guard let dir = findModelFolder(root, containing: "streaming_encoder.mlmodelc") else {
            throw SttError.notInstalled
        }
        let manager = StreamingEouAsrManager(
            configuration: aneConfiguration(),
            chunkSize: variant.eouChunkSize ?? .ms320,
            eouDebounceMs: EOU_DEBOUNCE_MS)
        try await manager.loadModels(from: dir)
        return .streaming(manager)
    case "nemotron":
        // The tier folder's metadata.json carries the chunk configuration, so
        // the manager self-configures to whichever tier was downloaded.
        let dir = findModelFolder(root, containing: "metadata.json")
            ?? findWhisperModelFolder(root) ?? root
        let manager = StreamingNemotronAsrManager()
        try await manager.loadModels(from: dir)
        return .streaming(manager)
    default:
        throw SttError.badEngine(entry.engine)
    }
}

/// Keeps the most-recently-loaded engine resident in RAM so consecutive
/// captures — and the prewarm that precedes the first — reuse a model already
/// paged in, instead of re-instantiating multi-GB CoreML weights on every
/// `start-capture`. This is the difference between a ~1 s warm start and the
/// instant mic-hot that the always-on dictation promise needs; the Rust side
/// keeps the serve process alive across captures so this cache survives
/// between them (a throwaway process per capture would defeat it).
///
/// One entry, keyed by `(modelsDir, modelId)`: dictation uses one model at a
/// time, and a model switch in Settings simply re-keys (the old engine drops,
/// freeing its RAM). An actor so the serve loop's sequential `handle` calls
/// and any future concurrent caller race safely on the slot.
actor EngineCache {
    static let shared = EngineCache()

    private var key: String?
    private var engine: LoadedEngine?
    /// In-flight load, keyed like `key`. Actor methods interleave at `await`,
    /// so without this two concurrent `resident` calls for the same model (a
    /// boot prewarm racing a mic-first capture) would page the multi-GB
    /// weights in twice — instead the second caller awaits the first's load.
    private var loading: (key: String, task: Task<LoadedEngine, Error>)?

    /// The resident engine for this model, loading + caching it on a miss.
    /// A different key evicts the previous engine before loading the new one,
    /// so only one model is ever held.
    func resident(modelsDir: String, entry: CatalogModel) async throws -> LoadedEngine {
        let wanted = modelsDir + "\u{1}" + entry.id
        if wanted == key, let engine {
            return engine
        }
        if let loading, loading.key == wanted {
            return try await loading.task.value
        }
        // Drop the old engine first so its CoreML weights are released before
        // the new load pages in (avoids briefly holding two multi-GB models).
        engine = nil
        key = nil
        let task = Task { try await loadEngine(modelsDir: modelsDir, entry: entry) }
        loading = (wanted, task)
        do {
            let loaded = try await task.value
            // Cache only if a different-model load didn't replace this one
            // while we were suspended.
            if loading?.key == wanted {
                engine = loaded
                key = wanted
                loading = nil
            }
            return loaded
        } catch {
            if loading?.key == wanted {
                loading = nil
            }
            throw error
        }
    }

    /// Release the resident engine (model deleted, or shutting down). The next
    /// `resident` call reloads from disk.
    func evict() {
        engine = nil
        key = nil
    }
}

enum SttError: Error, CustomStringConvertible {
    case notInstalled
    case badEngine(String)
    case captureActive
    case noCapture
    case micDenied

    var description: String {
        switch self {
        case .notInstalled: return "model is not installed"
        case .badEngine(let engine): return "unknown engine: \(engine)"
        case .captureActive: return "a capture session is already active"
        case .noCapture: return "no capture session is active"
        case .micDenied: return "microphone access was denied"
        }
    }
}

// MARK: - Audio capture

/// AVAudioEngine mic capture → 16 kHz mono Float32 accumulator + RMS levels.
/// The tap callback runs on Core Audio's thread; the lock guards the sample
/// buffer against the transcription passes reading snapshots.
final class AudioCapture: @unchecked Sendable {
    /// Optional only so `stop()` can hand the engine to a background queue
    /// for deallocation: tearing down an `AVAudioEngine` whose device just
    /// disappeared (Bluetooth drop, display-audio unplug) can block inside
    /// CoreAudio — releasing it off-thread keeps the serve loop responsive.
    private var engine: AVAudioEngine? = AVAudioEngine()
    private let lock = NSLock()
    private var samples: [Float] = []
    private var converter: AVAudioConverter?
    private var lastLevelSent = Date.distantPast

    static let sampleRate = 16_000.0
    /// Hard cap on the retained sample buffer: 30 minutes at 16 kHz mono
    /// Float32 (~115 MB). Normal dictation is seconds to a few minutes and
    /// never approaches this; the cap is a safety valve so a stuck or forgotten
    /// session can't grow the buffer (and the final full-buffer transcription
    /// pass) without bound. Enforced as an O(1) growth stop rather than a
    /// rolling trim so the Core Audio real-time tap never does a large memmove
    /// under the lock.
    static let maxSamples = Int(sampleRate) * 60 * 30

    func start() throws {
        guard let engine else {
            throw SttError.badEngine("audio engine already released")
        }
        let input = engine.inputNode
        let inputFormat = input.outputFormat(forBus: 0)
        guard
            let targetFormat = AVAudioFormat(
                commonFormat: .pcmFormatFloat32, sampleRate: Self.sampleRate,
                channels: 1, interleaved: false),
            let converter = AVAudioConverter(from: inputFormat, to: targetFormat)
        else {
            throw SttError.badEngine("audio format")
        }
        self.converter = converter

        input.installTap(onBus: 0, bufferSize: 4096, format: inputFormat) { [weak self] buffer, _ in
            self?.ingest(buffer, targetFormat: targetFormat)
        }
        engine.prepare()
        try engine.start()
    }

    func stop() {
        guard let engine else { return }
        engine.inputNode.removeTap(onBus: 0)
        engine.stop()
        self.engine = nil
        DispatchQueue.global(qos: .utility).async {
            _ = engine
        }
    }

    /// Snapshot of everything captured so far.
    func snapshot() -> [Float] {
        lock.lock()
        defer { lock.unlock() }
        return samples
    }

    private func ingest(_ buffer: AVAudioPCMBuffer, targetFormat: AVAudioFormat) {
        guard let converter else { return }
        // Output capacity scaled by the rate ratio, +1 frame of headroom.
        let ratio = Self.sampleRate / buffer.format.sampleRate
        let capacity = AVAudioFrameCount(Double(buffer.frameLength) * ratio) + 1
        guard let out = AVAudioPCMBuffer(pcmFormat: targetFormat, frameCapacity: capacity) else {
            return
        }
        var fed = false
        var error: NSError?
        converter.convert(to: out, error: &error) { _, status in
            if fed {
                status.pointee = .noDataNow
                return nil
            }
            fed = true
            status.pointee = .haveData
            return buffer
        }
        guard error == nil, let channel = out.floatChannelData, out.frameLength > 0 else { return }
        let chunk = Array(UnsafeBufferPointer(start: channel[0], count: Int(out.frameLength)))

        lock.lock()
        // Stop growing once the safety cap is hit (see `maxSamples`). Dropping
        // the newest audio is O(1) and keeps the real-time tap glitch-free;
        // real sessions never reach the cap.
        if samples.count < Self.maxSamples {
            samples.append(contentsOf: chunk)
        }
        let level = sqrt(chunk.reduce(0) { $0 + $1 * $1 } / Float(chunk.count))
        let levelDue = Date().timeIntervalSince(lastLevelSent) >= 0.15
        if levelDue { lastLevelSent = Date() }
        lock.unlock()

        if levelDue {
            LineWriter.shared.send(Event(event: "level", rms: Double(level)))
        }
    }
}

// MARK: - Capture session

/// The one active capture: mic accumulator + loaded engine + the partial
/// cadence task. One session machine-wide (same contract as macOS
/// dictation); `start-capture` while active is a bad request.
actor CaptureSession {
    static let shared = SessionSlot()

    /// Single-slot holder — an actor so the serve loop and capture tasks
    /// race safely on start/stop/cancel.
    actor SessionSlot {
        private var session: CaptureSession?

        func begin(_ new: CaptureSession) throws {
            guard session == nil else { throw SttError.captureActive }
            session = new
        }

        func take() throws -> CaptureSession {
            guard let current = session else { throw SttError.noCapture }
            session = nil
            return current
        }

        func clear() {
            session = nil
        }

        /// Clear only if `candidate` still owns the slot — the engine-attach
        /// failure path must not evict a successor session that already
        /// replaced this one via stop/cancel + a fresh start.
        func clear(ifCurrent candidate: CaptureSession) {
            if session === candidate {
                session = nil
            }
        }
    }

    private let capture = AudioCapture()
    /// The engine, possibly STILL LOADING when the mic goes hot: mic-first
    /// capture starts buffering immediately and `attachEngine` wires the
    /// decode loops when the load resolves, so a cold start (first dictation
    /// after launch) can't eat the user's first words.
    private let engineTask: Task<LoadedEngine, Error>
    /// Recognizer bias terms, tokenized once the engine arrives.
    private let biasTerms: [String]
    /// Resolved by `attachEngine`; nil while loading (or after a failed load).
    private var engine: LoadedEngine?
    /// Recognizer bias prompt tokens, computed once the engine attaches (nil
    /// for engines/inputs without a bias). Reused for every partial + the
    /// final pass so the whole session is biased identically.
    private var promptTokens: [Int]?
    /// Set by finish/cancel — a late engine attach must not start loops, and
    /// an engine-load failure after teardown has nothing left to clean up.
    private var torndown = false
    /// How much partial audio Whisper re-reads per pass: its native window.
    /// Parakeet TDT re-reads everything — at ~110× realtime a full pass is
    /// cheaper than Whisper's single window.
    private let partialWindowSeconds = 25.0
    private var partialTask: Task<Void, Never>?

    /// Incremental commit (Whisper only). Audio older than `commitLagSeconds`
    /// behind the live edge is transcribed once and frozen into `committedText`,
    /// so the release-time `finish()` pass only re-reads the short trailing tail
    /// instead of the whole buffer — the long-dictation latency win. Parakeet
    /// keeps `commitBoundary == 0` and finalizes the whole buffer (already
    /// ~110× realtime, so a full pass is cheap and stitching buys nothing).
    private let commitLagSeconds = 3.0
    /// Don't bother committing fewer than ~1 s of new stable audio.
    private let minCommitSamples = 16_000
    /// A VAD segment is only safe to freeze if it ended at least this long
    /// before the window edge — i.e. real silence followed it, so we're not
    /// cutting a word still in progress.
    private let commitTrailingGapSeconds = 0.25
    private var committedText = ""
    /// Sample offset such that `audio[0..<commitBoundary]` == `committedText`.
    private var commitBoundary = 0
    /// Re-entrancy guard: actor methods interleave at `await`, so this stops a
    /// commit pass and a partial pass both advancing state across a suspension.
    private var committing = false
    private var isWhisper: Bool {
        if case .whisper? = engine { return true }
        return false
    }

    /// The true-streaming manager when this session runs a cache-aware
    /// engine (Parakeet EOU / Nemotron); nil keeps the re-transcribe path.
    private var streamingManager: (any StreamingAsrManager)?
    /// How many accumulated samples have been fed to the streaming manager —
    /// the feed loop appends only the delta each tick.
    private var fedSamples = 0
    /// Consecutive failed streaming feed ticks — surfaced (once) so a
    /// degraded manager can't eat audio silently.
    private var streamFeedFailures = 0
    /// Last partial sent, to skip duplicate lines while the speaker pauses.
    private var lastStreamPartial = ""

    init(engineTask: Task<LoadedEngine, Error>, biasTerms: [String]) {
        self.engineTask = engineTask
        self.biasTerms = biasTerms
    }

    /// Rate-limits the `eou` event so a chatty End-of-Utterance detector
    /// (it can re-flag across consecutive silent chunks) emits at most one
    /// line per second. The app gates what an `eou` means (hands-free
    /// auto-stop, preference-controlled); the sidecar just reports.
    private final class EouGate: @unchecked Sendable {
        private let lock = NSLock()
        private var last = Date.distantPast
        func fire() {
            lock.lock()
            defer { lock.unlock() }
            guard Date().timeIntervalSince(last) >= 1.0 else { return }
            last = Date()
            LineWriter.shared.send(Event(event: "eou"))
        }
    }

    /// Mic-hot NOW, before the engine resolves: start buffering and tell the
    /// app — `listening` is its mic-hot signal, so the notch goes live and a
    /// cold model load can't lose what the user says in the meantime.
    func startListening() throws {
        try capture.start()
        LineWriter.shared.send(Event(event: "listening"))
    }

    /// Second half of the mic-first start: await the (possibly cold) engine
    /// load, then wire the decode loops behind the already-running mic. A
    /// load failure stops the mic and rethrows for the handler to report; a
    /// session finished/cancelled mid-load returns quietly (the resolved
    /// engine stays cached for the next session).
    func attachEngine() async throws {
        let engine: LoadedEngine
        do {
            engine = try await engineTask.value
        } catch {
            failBehindMic()
            throw error
        }
        guard !torndown else { return }
        self.engine = engine
        self.promptTokens = engine.biasPromptTokens(biasTerms)
        if case .streaming(let manager) = engine {
            // Clean decoder/cache state from any prior session on this
            // resident engine, and hook EOU before the loop feeds audio.
            do {
                try await manager.reset()
            } catch {
                failBehindMic()
                throw error
            }
            if let eou = manager as? StreamingEouAsrManager {
                let gate = EouGate()
                await eou.setEouCallback { _ in gate.fire() }
            }
            // finish/cancel may have landed across the awaits above.
            guard !torndown else { return }
            self.streamingManager = manager
            // Feed cadence: ~4×/s keeps the manager's internal chunker fed
            // (320 ms tiers decode roughly every other tick) and the overlay
            // partial fresh; a slow pass just delays the next tick. The first
            // tick's delta is everything buffered during the load.
            partialTask = Task { [weak self] in
                while !Task.isCancelled {
                    try? await Task.sleep(nanoseconds: 250_000_000)
                    if Task.isCancelled { break }
                    guard let self else { break }
                    await self.pumpStreaming(manager)
                }
            }
            return
        }
        partialTask = Task { [weak self] in
            // Re-transcribe cadence: 2 s of silence-tolerance keeps the
            // overlay honest without saturating the ANE. A pass that takes
            // longer than the cadence simply delays the next one — the loop
            // never stacks passes.
            var lastCount = 0
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 2_000_000_000)
                if Task.isCancelled { break }
                guard let self else { break }
                // Freeze stable audio first (Whisper only), then show the live
                // tail on top of the frozen prefix.
                await self.commitStable()
                lastCount = await self.emitPartial(ifGrownPast: lastCount)
            }
        }
    }

    /// Engine setup failed behind a live mic: release it and tell the app
    /// the session ended (the handler's failure response says why). No-op
    /// when finish/cancel already tore the session down.
    private func failBehindMic() {
        guard !torndown else { return }
        torndown = true
        capture.stop()
        LineWriter.shared.send(Event(event: "ended"))
    }

    /// One streaming tick: hand the manager the new audio since last tick,
    /// let it decode any complete chunks, and surface the accumulated
    /// hypothesis. The work-per-tick is bounded by the tick interval (only
    /// the delta is appended), so release-time latency stays O(one chunk)
    /// no matter how long the dictation ran.
    private func pumpStreaming(_ manager: any StreamingAsrManager) async {
        let all = capture.snapshot()
        guard all.count > fedSamples else { return }
        let delta = Array(all[fedSamples..<all.count])
        guard let buffer = makePCMBuffer(delta) else { return }
        do {
            try await manager.appendAudio(buffer)
            try await manager.processBufferedAudio()
            // Advance only after the manager accepted the chunk — advancing
            // before a throw would permanently skip this delta's audio.
            fedSamples = all.count
            streamFeedFailures = 0
        } catch {
            // Recoverable: fedSamples didn't advance, so the next tick
            // re-feeds the same delta from the accumulator. A degraded
            // manager must not eat audio silently — say so once per run.
            streamFeedFailures += 1
            if streamFeedFailures == 8 {
                FileHandle.standardError.write(
                    Data(
                        "portbay-stt: streaming feed failing repeatedly (\(streamFeedFailures) ticks): \(error)\n"
                            .utf8))
            }
            return
        }
        let text = await manager.getPartialTranscript()
            .trimmingCharacters(in: .whitespacesAndNewlines)
        if !text.isEmpty, text != lastStreamPartial {
            lastStreamPartial = text
            LineWriter.shared.send(Event(event: "partial", text: text))
        }
    }

    /// Join the frozen prefix to a freshly-transcribed tail for display/final.
    private func joinCommitted(with tail: String) -> String {
        let t = tail.trimmingCharacters(in: .whitespacesAndNewlines)
        if committedText.isEmpty { return t }
        if t.isEmpty { return committedText }
        return committedText + " " + t
    }

    /// Freeze stable audio (Whisper only). Transcribes the uncommitted region up
    /// to `commitLagSeconds` behind the live edge, commits whole VAD segments
    /// that ended on silence, and advances `commitBoundary` only to the last
    /// such segment's end — so a word in progress at the window edge stays in
    /// the tail for next time. No-op for Parakeet, and skipped while a pass is
    /// already in flight.
    private func commitStable() async {
        guard let engine, isWhisper, !committing else { return }
        let all = capture.snapshot()
        let lag = Int(commitLagSeconds * AudioCapture.sampleRate)
        let stableEnd = all.count - lag
        guard stableEnd - commitBoundary >= minCommitSamples else { return }
        let window = Array(all[commitBoundary..<stableEnd])
        committing = true
        defer { committing = false }
        guard
            let segments = try? await engine.whisperSegments(window, promptTokens: promptTokens),
            !segments.isEmpty
        else { return }
        // Keep only segments that clearly ended before the window edge (silence
        // after them); a segment running to the edge may be mid-word.
        let windowDur = Double(window.count) / AudioCapture.sampleRate
        let safe = segments.filter { Double($0.end) <= windowDur - commitTrailingGapSeconds }
        guard let last = safe.last else { return }
        let raw = safe.map(\.text).joined()
        // Same silence-hallucination guard the final pass uses.
        let text = filterWhisperHallucination(raw, segments: safe)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        committedText = committedText.isEmpty ? text : committedText + " " + text
        commitBoundary += Int(Double(last.end) * AudioCapture.sampleRate)
    }

    /// One partial pass for the overlay. Whisper transcribes only the
    /// uncommitted tail and shows it on top of the frozen `committedText`;
    /// Parakeet re-reads the whole buffer. Returns the sample count it saw, so
    /// the loop skips passes while the mic is silent (count unchanged).
    private func emitPartial(ifGrownPast lastCount: Int) async -> Int {
        guard let engine else { return lastCount }
        let all = capture.snapshot()
        guard all.count > lastCount, all.count >= Int(AudioCapture.sampleRate / 2) else {
            return lastCount
        }
        let window: [Float]
        if isWhisper {
            let tail = Array(all[min(commitBoundary, all.count)..<all.count])
            // Tail too short to transcribe cleanly — just show what's frozen.
            if tail.count < Int(AudioCapture.sampleRate / 2) {
                if !committedText.isEmpty {
                    LineWriter.shared.send(Event(event: "partial", text: committedText))
                }
                return all.count
            }
            let cap = Int(partialWindowSeconds * AudioCapture.sampleRate)
            window = tail.count > cap ? Array(tail.suffix(cap)) : tail
        } else {
            window = all
        }
        if let text = try? await engine.transcribe(window, finalPass: false, promptTokens: promptTokens) {
            let combined = isWhisper ? joinCommitted(with: text) : text
            if !combined.isEmpty {
                LineWriter.shared.send(Event(event: "partial", text: combined))
            }
        }
        return all.count
    }

    /// Stop the mic, finalize only the uncommitted tail (Whisper), the last
    /// partial chunk (streaming engines), or the whole buffer (Parakeet TDT),
    /// emit final + ended. For Whisper the frozen `committedText` — and for
    /// streaming engines the chunk-by-chunk decode that already happened —
    /// means release-time latency tracks tail length, never total length.
    func finish() async -> String {
        torndown = true
        partialTask?.cancel()
        partialTask = nil
        capture.stop()
        // Mic-first: the engine may still be loading — the words are already
        // safe in the accumulator, so wait the load out and transcribe. A
        // failed load has nothing to decode with; the start-capture failure
        // response (and the app's teardown on it) carries the reason.
        var resolved = self.engine
        if resolved == nil {
            resolved = try? await engineTask.value
        }
        guard let engine = resolved else {
            LineWriter.shared.send(Event(event: "final", text: ""))
            LineWriter.shared.send(Event(event: "ended"))
            return ""
        }
        // Stopped before `attachEngine` finished wiring: tokenize the bias
        // now so the one-shot final pass below is biased like a live session.
        if promptTokens == nil {
            promptTokens = engine.biasPromptTokens(biasTerms)
        }
        if case .streaming(let manager) = engine {
            // attachEngine never ran (stopped mid-load): clean state first —
            // the whole buffer is then the un-fed delta below.
            if streamingManager == nil {
                try? await manager.reset()
            }
            // Feed whatever landed after the last tick, then flush: the
            // manager pads + decodes only the final partial chunk (~tens of
            // ms), everything earlier was decoded live.
            let all = capture.snapshot()
            if all.count > fedSamples, let buffer = makePCMBuffer(Array(all[fedSamples..<all.count])) {
                try? await manager.appendAudio(buffer)
            }
            fedSamples = all.count
            let flushed = (try? await manager.finish())?
                .trimmingCharacters(in: .whitespacesAndNewlines)
            // A failed flush still has the live hypothesis — degrade to it
            // rather than dropping the user's words.
            let full = (flushed?.isEmpty == false) ? flushed! : lastStreamPartial
            LineWriter.shared.send(Event(event: "final", text: full))
            LineWriter.shared.send(Event(event: "ended"))
            return full
        }
        let all = capture.snapshot()
        let tailStart = min(commitBoundary, all.count)
        let tail = Array(all[tailStart..<all.count])
        var tailText = ""
        if tail.count >= Int(AudioCapture.sampleRate / 4) {
            tailText = (try? await engine.transcribe(tail, finalPass: true, promptTokens: promptTokens)) ?? ""
        }
        let full = joinCommitted(with: tailText)
        LineWriter.shared.send(Event(event: "final", text: full))
        LineWriter.shared.send(Event(event: "ended"))
        return full
    }

    /// Tear down without a final pass (the words are discarded by design).
    func cancel() {
        torndown = true
        partialTask?.cancel()
        partialTask = nil
        capture.stop()
        if let manager = streamingManager {
            // Clear decoder/cache state so the discarded words can't leak
            // into the next session on this resident engine (start() resets
            // too — this just frees the state promptly).
            Task { try? await manager.reset() }
        }
        LineWriter.shared.send(Event(event: "ended"))
    }
}

/// Mic permission, resolved before the engine spins up. TCC attributes the
/// prompt/grant to the responsible app (PortBay), not this helper.
func requestMicAccess() async -> Bool {
    await withCheckedContinuation { continuation in
        AVCaptureDevice.requestAccess(for: .audio) { granted in
            continuation.resume(returning: granted)
        }
    }
}

// MARK: - --check

// MARK: - --digest (maintainer tool)

// Print the canonical content digest of an installed model directory —
// `portbay-stt --digest <modelsDir>/<id>`. This is how the live PortBay
// Model Catalog gets its `contentDigest` values: download the model once on
// a trusted machine, digest it here, publish the hash in the signed
// manifest. Clients then verify every download against it before sealing.
if let i = CommandLine.arguments.firstIndex(of: "--digest"),
    CommandLine.arguments.indices.contains(i + 1)
{
    let root = URL(fileURLWithPath: CommandLine.arguments[i + 1], isDirectory: true)
    do {
        print(try directoryContentDigest(root))
        exit(0)
    } catch {
        FileHandle.standardError.write(Data("portbay-stt: digest failed: \(error)\n".utf8))
        exit(1)
    }
}

if CommandLine.arguments.contains("--check") {
    // The deployment target (macOS 14) already gates launch — if this code
    // runs, the engines' platform floor is met. Touch one symbol from each
    // engine library so a build that silently dropped a dependency fails
    // here, at the status probe, not at first capture.
    let engines = [
        "whisper:\(String(describing: WhisperKit.self))",
        "parakeet:\(String(describing: AsrManager.self))",
    ]
    let names = engines.map { $0.split(separator: ":").first.map(String.init) ?? $0 }
    let list = names.map { "\"\($0)\"" }.joined(separator: ",")
    print("{\"available\":true,\"engines\":[\(list)]}")
    exit(0)
}

// MARK: - --serve

func badRequest(_ op: String, _ detail: String) -> Response {
    Response(op: op, ok: false, code: 4, error: detail)
}

/// Handle one request. Phase 1 implements the model-metadata ops (catalog,
/// installed, delete); download and capture land with their backing engine
/// plumbing in the next phases and answer code 2 until then so the app's
/// status surfaces stay honest.
func handle(_ request: Request) async {
    let out = LineWriter.shared
    switch request.op {
    case "catalog":
        out.send(Response(op: "catalog", ok: true, models: CATALOG))

    case "installed":
        guard let dir = request.modelsDir, !dir.isEmpty else {
            out.send(badRequest("installed", "modelsDir is required"))
            return
        }
        out.send(Response(op: "installed", ok: true, installed: scanInstalled(modelsDir: dir)))

    case "delete":
        guard let dir = request.modelsDir, !dir.isEmpty, let model = request.model, !model.isEmpty
        else {
            out.send(badRequest("delete", "modelsDir and model are required"))
            return
        }
        // No catalog membership check: the live PortBay Model Catalog can list
        // models the bundled CATALOG doesn't, and a sealed install on disk is
        // authoritative. Reject only a traversal-shaped id.
        guard !model.contains("/"), !model.contains("..") else {
            out.send(badRequest("delete", "invalid model id: \(model)"))
            return
        }
        let target = (dir as NSString).appendingPathComponent(model)
        guard FileManager.default.fileExists(atPath: target) else {
            // Already gone — deleting an absent model is success, not error.
            out.send(Response(op: "delete", ok: true))
            return
        }
        do {
            try FileManager.default.removeItem(atPath: target)
            out.send(Response(op: "delete", ok: true))
        } catch {
            out.send(Response(op: "delete", ok: false, code: 5, error: "delete failed: \(error.localizedDescription)"))
        }

    case "download":
        guard let dir = request.modelsDir, !dir.isEmpty,
            let model = request.model, !model.isEmpty,
            let downloadId = request.downloadId, !downloadId.isEmpty
        else {
            out.send(badRequest("download", "modelsDir, model and downloadId are required"))
            return
        }
        guard let entry = resolveEntry(modelsDir: dir, request: request) else {
            out.send(badRequest("download", "unknown model id: \(model)"))
            return
        }
        // Detached so the serve loop keeps reading (cancel-download must be
        // able to land mid-download). The task sends its own terminal line.
        // Route by engine: Whisper/Parakeet, the advanced FluidAudio ASR
        // engines, and TTS (Kokoro) each have their own downloader.
        let engine = entry.engine
        let task = Task.detached {
            switch engine {
            case "kokoro":
                await runTtsDownload(modelsDir: dir, entry: entry, downloadId: downloadId)
            case "qwen3", "cohere", "nemotron", "parakeet-eou":
                await runAdvancedDownload(modelsDir: dir, entry: entry, downloadId: downloadId)
            default:
                await runDownload(modelsDir: dir, entry: entry, downloadId: downloadId)
            }
        }
        await ActiveDownloads.shared.register(downloadId, task)

    case "cancel-download":
        guard let downloadId = request.downloadId, !downloadId.isEmpty else {
            out.send(badRequest("cancel-download", "downloadId is required"))
            return
        }
        await ActiveDownloads.shared.cancel(downloadId)
        out.send(Response(op: "cancel-download", ok: true, downloadId: downloadId))

    case "prewarm":
        guard let dir = request.modelsDir, !dir.isEmpty, request.model?.isEmpty == false,
            let entry = resolveEntry(modelsDir: dir, request: request)
        else {
            out.send(badRequest("prewarm", "modelsDir and a known model are required"))
            return
        }
        // Load + KEEP resident: pages weights in, re-validates CoreML's
        // specialization cache (evicted on OS updates), and holds the
        // engine in `EngineCache` so the next `start-capture` reuses it
        // for an instant mic-hot instead of reloading. The Rust side keeps
        // this serve process alive across captures so the residency lasts.
        // Detached so the serve loop keeps reading while a cold multi-GB
        // load runs: a mic-first start-capture must be able to land mid-load
        // (its own `resident` call coalesces onto this load). Responses are
        // op-keyed on the Rust side, so an out-of-order terminal is fine.
        Task {
            do {
                _ = try await EngineCache.shared.resident(modelsDir: dir, entry: entry)
                out.send(Response(op: "prewarm", ok: true))
            } catch {
                out.send(Response(op: "prewarm", ok: false, code: 2, error: "\(error)"))
            }
        }

    case "start-capture":
        guard let dir = request.modelsDir, !dir.isEmpty, request.model?.isEmpty == false,
            let entry = resolveEntry(modelsDir: dir, request: request)
        else {
            out.send(badRequest("start-capture", "modelsDir and a known model are required"))
            return
        }
        do {
            guard await requestMicAccess() else { throw SttError.micDenied }
            // Mic-first: the capture starts buffering (and `listening`
            // resolves the app's start) BEFORE the engine resolves, so a cold
            // model load — first dictation after launch — can't eat the
            // user's first words. The load coalesces with any in-flight boot
            // prewarm via EngineCache and attaches behind the live mic.
            let engineTask = Task {
                try await EngineCache.shared.resident(modelsDir: dir, entry: entry)
            }
            let session = CaptureSession(engineTask: engineTask, biasTerms: request.biasTerms ?? [])
            try await CaptureSession.shared.begin(session)
            do {
                try await session.startListening()
            } catch {
                await CaptureSession.shared.clear()
                throw error
            }
            // Detached: the serve loop must keep reading (stop/cancel may
            // land mid-load and are ordered behind the slot, not this task).
            // The failure response doubles as the app's signal that a
            // mic-first session died behind the `listening` it already saw.
            Task {
                do {
                    try await session.attachEngine()
                } catch {
                    await CaptureSession.shared.clear(ifCurrent: session)
                    out.send(Response(op: "start-capture", ok: false, code: 2, error: "\(error)"))
                }
            }
            // No terminal line on success — the `listening` event is the
            // confirmation; the terminal comes from stop/cancel.
        } catch {
            out.send(Response(op: "start-capture", ok: false, code: 2, error: "\(error)"))
        }

    case "stop-capture":
        do {
            let session = try await CaptureSession.shared.take()
            _ = await session.finish()
            out.send(Response(op: "stop-capture", ok: true))
        } catch {
            out.send(Response(op: "stop-capture", ok: false, code: 4, error: "\(error)"))
        }

    case "cancel-capture":
        do {
            let session = try await CaptureSession.shared.take()
            await session.cancel()
            out.send(Response(op: "cancel-capture", ok: true))
        } catch {
            out.send(Response(op: "cancel-capture", ok: false, code: 4, error: "\(error)"))
        }

    case "tts-synthesize":
        guard let dir = request.modelsDir, !dir.isEmpty,
            let model = request.model, !model.isEmpty,
            let text = request.text, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else {
            out.send(badRequest("tts-synthesize", "modelsDir, model and text are required"))
            return
        }
        do {
            let wav = try await synthesizeTts(
                modelsDir: dir, id: model, text: text, voice: request.voice)
            out.send(Response(op: "tts-synthesize", ok: true, wavBase64: wav))
        } catch {
            out.send(Response(op: "tts-synthesize", ok: false, code: 2, error: "\(error)"))
        }

    default:
        out.send(badRequest(request.op, "unknown op: \(request.op)"))
    }
}

if CommandLine.arguments.contains("--serve") {
    // Release the resident model when macOS signals **critical** memory
    // pressure, so an idle multi-GB engine never starves the system (or
    // invites a jetsam kill of the app it belongs to) — it transparently
    // reloads on the next capture. This is what keeps "always resident for
    // instant dictation" production-safe: instant by default, but a good
    // citizen exactly when RAM is scarce. `.critical` only — `.warning` is
    // routine and evicting on it would thrash the reload. Held for the serve
    // process's lifetime (the `while` loop below keeps the binding alive).
    let memoryPressureSource = DispatchSource.makeMemoryPressureSource(
        eventMask: .critical, queue: .global(qos: .utility))
    memoryPressureSource.setEventHandler {
        Task { await EngineCache.shared.evict() }
    }
    memoryPressureSource.activate()

    // Request loop: read a line, dispatch. A malformed line gets an error
    // response and the loop continues — only stdin EOF (the app closed the
    // pipe or died) ends the server.
    while let line = readLine(strippingNewline: true) {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        if trimmed.isEmpty { continue }
        guard let request = try? JSONDecoder().decode(Request.self, from: Data(trimmed.utf8)) else {
            LineWriter.shared.send(Response(op: "?", ok: false, code: 4, error: "invalid request line"))
            continue
        }
        await handle(request)
    }
    exit(0)
}

FileHandle.standardError.write(Data("portbay-stt: use --check or --serve\n".utf8))
exit(4)
