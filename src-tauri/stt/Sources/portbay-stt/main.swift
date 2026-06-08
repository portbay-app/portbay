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
}

/// Curated, static — STT models don't live in any registry PortBay could
/// query (Ollama's library has zero STT models), so PortBay ships its own
/// shortlist. Order is display order. Sizes verified against the HF repos
/// when download lands (Phase 2); until then they are honest approximations.
let CATALOG: [CatalogModel] = [
    CatalogModel(
        id: "parakeet-tdt-v3",
        engine: "parakeet",
        displayName: "Parakeet TDT v3 (0.6B)",
        repoModel: "parakeet-tdt-0.6b-v3-coreml",
        approxSizeBytes: 2_400_000_000,
        languages: "25 European languages",
        speedNote: "Fastest on Apple Silicon — near-instant transcription on the Neural Engine.",
        recommended: true,
        streaming: false
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

/// Per-model install root: `<modelsDir>/<catalog id>/`. Each engine library
/// is pointed inside this folder, so delete is one directory removal.
func modelRoot(_ modelsDir: String, _ id: String) -> URL {
    URL(fileURLWithPath: modelsDir).appendingPathComponent(id, isDirectory: true)
}

/// A model is installed when its root exists AND carries the completion
/// marker.
func scanInstalled(modelsDir: String) -> [InstalledModel] {
    let fm = FileManager.default
    return CATALOG.compactMap { entry in
        let root = modelRoot(modelsDir, entry.id)
        guard fm.fileExists(atPath: root.appendingPathComponent(COMPLETE_MARKER).path) else {
            return nil
        }
        return InstalledModel(id: entry.id, engine: entry.engine, sizeBytes: directorySize(root.path))
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
            _ = try await AsrModels.download(
                to: target,
                version: .v3,
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

enum LoadedEngine {
    case whisper(WhisperKit)
    case parakeet(AsrManager)

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
        }
    }
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
        let models = try await AsrModels.load(from: target)
        let manager = AsrManager(config: .default)
        try await manager.loadModels(models)
        return .parakeet(manager)
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

    /// The resident engine for this model, loading + caching it on a miss.
    /// A different key evicts the previous engine before loading the new one,
    /// so only one model is ever held.
    func resident(modelsDir: String, entry: CatalogModel) async throws -> LoadedEngine {
        let wanted = modelsDir + "\u{1}" + entry.id
        if wanted == key, let engine {
            return engine
        }
        // Drop the old engine first so its CoreML weights are released before
        // the new load pages in (avoids briefly holding two multi-GB models).
        engine = nil
        key = nil
        let loaded = try await loadEngine(modelsDir: modelsDir, entry: entry)
        engine = loaded
        key = wanted
        return loaded
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
    private let engine = AVAudioEngine()
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
        engine.inputNode.removeTap(onBus: 0)
        engine.stop()
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
    }

    private let capture = AudioCapture()
    private let engine: LoadedEngine
    /// Recognizer bias prompt tokens, computed once at session start (nil for
    /// engines/inputs without a bias). Reused for every partial + the final
    /// pass so the whole session is biased identically.
    private let promptTokens: [Int]?
    /// How much partial audio Whisper re-reads per pass: its native window.
    /// Parakeet TDT re-reads everything — at ~110× realtime a full pass is
    /// cheaper than Whisper's single window.
    private let partialWindowSeconds = 25.0
    private var partialTask: Task<Void, Never>?

    init(engine: LoadedEngine, promptTokens: [Int]? = nil) {
        self.engine = engine
        self.promptTokens = promptTokens
    }

    func start() throws {
        try capture.start()
        LineWriter.shared.send(Event(event: "listening"))
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
                lastCount = await self.emitPartial(ifGrownPast: lastCount)
            }
        }
    }

    /// One partial pass. Returns the sample count it saw, so the loop skips
    /// passes while the mic is silent (count unchanged).
    private func emitPartial(ifGrownPast lastCount: Int) async -> Int {
        let all = capture.snapshot()
        // Sub-half-second audio transcribes to noise; wait for real speech.
        guard all.count > lastCount, all.count >= Int(AudioCapture.sampleRate / 2) else {
            return lastCount
        }
        let window: [Float]
        if case .whisper = engine {
            let cap = Int(partialWindowSeconds * AudioCapture.sampleRate)
            window = all.count > cap ? Array(all.suffix(cap)) : all
        } else {
            window = all
        }
        if let text = try? await engine.transcribe(window, finalPass: false, promptTokens: promptTokens),
            !text.isEmpty
        {
            LineWriter.shared.send(Event(event: "partial", text: text))
        }
        return all.count
    }

    /// Stop the mic, run the final full-buffer pass, emit final + ended.
    func finish() async -> String {
        partialTask?.cancel()
        partialTask = nil
        capture.stop()
        let all = capture.snapshot()
        var text = ""
        if all.count >= Int(AudioCapture.sampleRate / 4) {
            text = (try? await engine.transcribe(all, finalPass: true, promptTokens: promptTokens)) ?? ""
        }
        LineWriter.shared.send(Event(event: "final", text: text))
        LineWriter.shared.send(Event(event: "ended"))
        return text
    }

    /// Tear down without a final pass (the words are discarded by design).
    func cancel() {
        partialTask?.cancel()
        partialTask = nil
        capture.stop()
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
        guard CATALOG.contains(where: { $0.id == model }) else {
            out.send(badRequest("delete", "unknown model id: \(model)"))
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
        guard let entry = CATALOG.first(where: { $0.id == model }) else {
            out.send(badRequest("download", "unknown model id: \(model)"))
            return
        }
        // Detached so the serve loop keeps reading (cancel-download must be
        // able to land mid-download). The task sends its own terminal line.
        let task = Task.detached {
            await runDownload(modelsDir: dir, entry: entry, downloadId: downloadId)
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
        guard let dir = request.modelsDir, !dir.isEmpty, let model = request.model, !model.isEmpty,
            let entry = CATALOG.first(where: { $0.id == model })
        else {
            out.send(badRequest("prewarm", "modelsDir and a known model are required"))
            return
        }
        do {
            // Load + KEEP resident: pages weights in, re-validates CoreML's
            // specialization cache (evicted on OS updates), and holds the
            // engine in `EngineCache` so the next `start-capture` reuses it
            // for an instant mic-hot instead of reloading. The Rust side keeps
            // this serve process alive across captures so the residency lasts.
            _ = try await EngineCache.shared.resident(modelsDir: dir, entry: entry)
            out.send(Response(op: "prewarm", ok: true))
        } catch {
            out.send(Response(op: "prewarm", ok: false, code: 2, error: "\(error)"))
        }

    case "start-capture":
        guard let dir = request.modelsDir, !dir.isEmpty, let model = request.model, !model.isEmpty,
            let entry = CATALOG.first(where: { $0.id == model })
        else {
            out.send(badRequest("start-capture", "modelsDir and a known model are required"))
            return
        }
        do {
            guard await requestMicAccess() else { throw SttError.micDenied }
            // Reuse the resident engine when prewarm (or a prior capture)
            // already loaded this model — the fast path that makes Fn-hold
            // instant; a cold miss loads + caches it here.
            let engine = try await EngineCache.shared.resident(modelsDir: dir, entry: entry)
            // Recognizer bias: tokenize the resolved terms for the Whisper
            // decoder; nil for Parakeet (graceful degrade — the rewrite layer
            // still corrects spellings downstream).
            let promptTokens = engine.biasPromptTokens(request.biasTerms ?? [])
            let session = CaptureSession(engine: engine, promptTokens: promptTokens)
            try await CaptureSession.shared.begin(session)
            do {
                try await session.start()
            } catch {
                await CaptureSession.shared.clear()
                throw error
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
