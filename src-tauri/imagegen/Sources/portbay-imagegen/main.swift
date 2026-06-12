// portbay-imagegen — local image-generation bridge for the AI page.
//
// Why a sidecar: PortBay's on-device diffusion engine (apple/ml-stable-
// diffusion) is Swift-only — it runs Core ML models on the Neural Engine / GPU
// through Swift APIs the Rust app cannot reach. This CLI is the bridge, modeled
// exactly on portbay-stt: line-delimited JSON over stdin/stdout, spawned and
// owned by src-tauri/src/imagegen.rs.
//
// Protocol:
//   portbay-imagegen --check
//     → stdout: one-line JSON {"available":Bool,"reason":String?,
//       "engines":[String]?}; exit 0. The binary only launches on macOS 14+
//       (deployment target) — the app maps a failed exec to requires_macos_14.
//   portbay-imagegen --serve
//     ← stdin:  one JSON request per line: {"op":String, ...op fields}.
//     → stdout: zero or more event lines {"event":String, ...} then exactly one
//       terminal response line {"op":String,"ok":Bool, ...} per request. EOF on
//       stdin = shutdown (exit 0).
//
//   Ops (modelsDir is passed on every request — the app owns the pref):
//     installed{modelsDir}              → {"op":"installed","ok":true,"installed":[InstalledModel]}
//     download{modelsDir,model,downloadId,engine,repoModel}
//       events {"event":"progress","fraction":Double,"phase":"downloading"}
//                                       → {"op":"download","ok":Bool,"downloadId":String,...}
//     cancel-download{downloadId}       → {"op":"cancel-download","ok":true}
//     delete{modelsDir,model}           → {"op":"delete","ok":Bool,...}
//     generate{modelsDir,model,prompt,negativePrompt?,steps?,guidance?,size?,seed?,engine,repoModel}
//       events {"event":"progress","fraction":Double,"step":Int,"totalSteps":Int}
//                                       → {"op":"generate","ok":true,"imageBase64":String}
//
//   Error responses: {"op":String,"ok":false,"code":Int,"error":String} —
//   codes mirror portbay-stt: 2 unavailable · 4 bad request · 5 failed ·
//   6 cancelled.
//
// NOTE: the actual diffusion call (`runDiffusion`) is the one integration seam
// against apple/ml-stable-diffusion — verified at first `swift build`, the same
// bring-up loop the STT sidecar went through. Everything else (protocol,
// download, install detection) is engine-agnostic.

import AppKit
import CoreML
import CryptoKit
import Foundation
import Hub
import StableDiffusion

// MARK: - Wire types

struct Request: Decodable {
    let op: String
    let modelsDir: String?
    let model: String?
    let downloadId: String?
    let engine: String?
    let repoModel: String?
    let prompt: String?
    let negativePrompt: String?
    let steps: Int?
    let guidance: Double?
    let size: Int?
    let seed: Int64?
    let compiledGlob: String?
    /// Expected install-content digest from the signed catalog (see
    /// `directoryContentDigest` in portbay-stt — same canonical format).
    /// Verified before the install is sealed; absent = no verification.
    let contentDigest: String?
}

struct InstalledModel: Encodable {
    let id: String
    let engine: String
    let sizeBytes: Int64
}

struct Response: Encodable {
    let op: String
    let ok: Bool
    var code: Int? = nil
    var error: String? = nil
    var installed: [InstalledModel]? = nil
    var downloadId: String? = nil
    var imageBase64: String? = nil
}

struct Event: Encodable {
    let event: String
    var fraction: Double? = nil
    var phase: String? = nil
    var step: Int? = nil
    var totalSteps: Int? = nil
}

// MARK: - Install layout (mirrors portbay-stt's marker/spec convention)

let COMPLETE_MARKER = ".portbay-complete"
let SPEC_MARKER = ".portbay-spec.json"

struct ModelSpec: Codable {
    let engine: String
    let repoModel: String
}

func modelRoot(_ modelsDir: String, _ id: String) -> URL {
    URL(fileURLWithPath: modelsDir).appendingPathComponent(id, isDirectory: true)
}

// MARK: - Install-content digest (mirrors portbay-stt)

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

/// Streaming SHA-256 of one file (weights run to multiple GB).
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
/// `relative-path:sha256` lines of every regular file (`.portbay-*` markers
/// excluded). Byte-compatible with portbay-stt's `--digest` output so one
/// maintainer flow produces hashes for both catalogs.
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

func writeSpec(_ root: URL, engine: String, repoModel: String) {
    let spec = ModelSpec(engine: engine, repoModel: repoModel)
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

func scanInstalled(modelsDir: String) -> [InstalledModel] {
    let fm = FileManager.default
    guard let ids = try? fm.contentsOfDirectory(atPath: modelsDir) else { return [] }
    return ids.compactMap { id in
        let root = modelRoot(modelsDir, id)
        guard fm.fileExists(atPath: root.appendingPathComponent(COMPLETE_MARKER).path) else {
            return nil
        }
        let engine = readSpec(root)?.engine ?? "sd"
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

/// The directory holding the compiled Core ML resources (`*.mlmodelc`) for the
/// pipeline. Apple's coreml-stable-diffusion repos nest these under e.g.
/// `split_einsum/compiled` or `original/compiled`; locate the folder that
/// actually contains `Unet.mlmodelc` rather than hardcoding the layout.
func resourcesDir(in root: URL) -> URL? {
    let fm = FileManager.default
    guard let walker = fm.enumerator(at: root, includingPropertiesForKeys: nil) else { return nil }
    for case let url as URL in walker where url.lastPathComponent == "Unet.mlmodelc" {
        return url.deletingLastPathComponent()
    }
    return nil
}

// MARK: - Serialized stdout writer

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
        let data =
            (try? JSONEncoder().encode(response))
            ?? Data(
                "{\"op\":\"\(response.op)\",\"ok\":false,\"code\":5,\"error\":\"encode failed\"}".utf8)
        writeLine(data)
    }

    func send(_ event: Event) {
        guard let data = try? JSONEncoder().encode(event) else { return }
        writeLine(data)
    }
}

// MARK: - Downloads

actor ActiveDownloads {
    static let shared = ActiveDownloads()
    private var tasks: [String: Task<Void, Never>] = [:]

    func register(_ id: String, _ task: Task<Void, Never>) { tasks[id] = task }
    func finish(_ id: String) { tasks[id] = nil }
    func cancel(_ id: String) {
        tasks[id]?.cancel()
        tasks[id] = nil
    }
}

/// Download a model's Core ML weights from Hugging Face into `<modelsDir>/<id>`,
/// emitting fractional progress, then seal it with the completion marker + spec.
func runDownload(_ req: Request) async {
    guard let modelsDir = req.modelsDir, let id = req.model,
        let engine = req.engine, let repo = req.repoModel, let downloadId = req.downloadId
    else {
        LineWriter.shared.send(Response(op: "download", ok: false, code: 4, error: "bad download request"))
        return
    }
    let root = modelRoot(modelsDir, id)
    try? FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)

    // Apple's repos differ in layout: SD 1.x/2.x carry both an `original` and a
    // Neural-Engine-optimized `split_einsum` variant (prefer the latter), while
    // `coreml-stable-diffusion-xl-base` ships its compiled resources directly
    // under a top-level `compiled/` (the `original` attention variant, no
    // variant prefix). Community conversions (e.g. SD-Turbo) ship only an
    // `original/compiled/`. The catalog can override the glob per model
    // (`compiledGlob`) so new layouts work without a sidecar rebuild; otherwise
    // derive it from the engine. Hub's `fnmatch` runs with flags 0, so `*` spans
    // `/` and each glob pulls the whole `*.mlmodelc` tree recursively.
    let glob = req.compiledGlob ?? (engine == "sdxl" ? "compiled/*" : "split_einsum/compiled/*")

    do {
        let hub = HubApi(downloadBase: root)
        _ = try await hub.snapshot(from: Hub.Repo(id: repo), matching: [glob]) { progress in
            LineWriter.shared.send(
                Event(event: "progress", fraction: progress.fractionCompleted, phase: "downloading"))
        }
        if Task.isCancelled {
            LineWriter.shared.send(Response(op: "download", ok: false, code: 6, error: "cancelled", downloadId: downloadId))
            await ActiveDownloads.shared.finish(downloadId)
            return
        }
        // Verify against the catalog's expected digest (no-op when absent)
        // BEFORE sealing — a corrupted/tampered download must never load.
        if let expected = req.contentDigest?.lowercased(), !expected.isEmpty {
            let actual = try directoryContentDigest(root)
            if actual != expected {
                throw DigestError.mismatch(expected: expected, actual: actual)
            }
        }
        // Seal: a half-download must never count as installed.
        FileManager.default.createFile(atPath: root.appendingPathComponent(COMPLETE_MARKER).path, contents: Data())
        writeSpec(root, engine: engine, repoModel: repo)
        LineWriter.shared.send(Response(op: "download", ok: true, downloadId: downloadId))
    } catch is CancellationError {
        LineWriter.shared.send(Response(op: "download", ok: false, code: 6, error: "cancelled", downloadId: downloadId))
    } catch {
        LineWriter.shared.send(
            Response(op: "download", ok: false, code: 5, error: "\(error)", downloadId: downloadId))
    }
    await ActiveDownloads.shared.finish(downloadId)
}

// MARK: - Generation

/// One resident pipeline, kept warm across generations (loading multi-GB Core
/// ML weights costs seconds). Evicted when the requested model changes.
final class PipelineCache: @unchecked Sendable {
    static let shared = PipelineCache()
    private var loadedId: String?
    private var sd: StableDiffusionPipeline?
    private var sdxl: StableDiffusionXLPipeline?

    func pipeline(id: String, engine: String, resources: URL) throws -> Any {
        if loadedId == id, let p = sd ?? nil { return p }
        if loadedId == id, let p = sdxl { return p }
        sd = nil
        sdxl = nil
        let config = MLModelConfiguration()
        config.computeUnits = .cpuAndNeuralEngine
        if engine == "sdxl" {
            let p = try StableDiffusionXLPipeline(
                resourcesAt: resources, configuration: config, reduceMemory: true)
            try p.loadResources()
            sdxl = p
            loadedId = id
            return p
        } else {
            let p = try StableDiffusionPipeline(
                resourcesAt: resources, controlNet: [], configuration: config,
                disableSafety: false, reduceMemory: true)
            try p.loadResources()
            sd = p
            loadedId = id
            return p
        }
    }
}

func pngBase64(_ image: CGImage) -> String? {
    let rep = NSBitmapImageRep(cgImage: image)
    guard let data = rep.representation(using: .png, properties: [:]) else { return nil }
    return data.base64EncodedString()
}

/// The single engine seam: run the diffusion pipeline and return a PNG.
func runDiffusion(_ req: Request) {
    guard let modelsDir = req.modelsDir, let id = req.model, let prompt = req.prompt else {
        LineWriter.shared.send(Response(op: "generate", ok: false, code: 4, error: "bad generate request"))
        return
    }
    let engine = req.engine ?? readSpec(modelRoot(modelsDir, id))?.engine ?? "sd"
    let root = modelRoot(modelsDir, id)
    guard FileManager.default.fileExists(atPath: root.appendingPathComponent(COMPLETE_MARKER).path),
        let resources = resourcesDir(in: root)
    else {
        LineWriter.shared.send(Response(op: "generate", ok: false, code: 2, error: "model not installed"))
        return
    }

    do {
        let steps = req.steps ?? 25
        let cg: CGImage?
        if engine == "sdxl" {
            let pipeline = try PipelineCache.shared.pipeline(id: id, engine: engine, resources: resources) as! StableDiffusionXLPipeline
            var cfg = StableDiffusionXLPipeline.Configuration(prompt: prompt)
            cfg.negativePrompt = req.negativePrompt ?? ""
            cfg.stepCount = steps
            if let g = req.guidance { cfg.guidanceScale = Float(g) }
            if let s = req.seed { cfg.seed = UInt32(truncatingIfNeeded: s) }
            cfg.imageCount = 1
            let images = try pipeline.generateImages(configuration: cfg) { progress in
                LineWriter.shared.send(
                    Event(event: "progress",
                        fraction: Double(progress.step) / Double(max(1, progress.stepCount)),
                        step: progress.step, totalSteps: progress.stepCount))
                return true
            }
            cg = images.compactMap { $0 }.first
        } else {
            let pipeline = try PipelineCache.shared.pipeline(id: id, engine: engine, resources: resources) as! StableDiffusionPipeline
            var cfg = StableDiffusionPipeline.Configuration(prompt: prompt)
            cfg.negativePrompt = req.negativePrompt ?? ""
            cfg.stepCount = steps
            if let g = req.guidance { cfg.guidanceScale = Float(g) }
            if let s = req.seed { cfg.seed = UInt32(truncatingIfNeeded: s) }
            cfg.imageCount = 1
            let images = try pipeline.generateImages(configuration: cfg) { progress in
                LineWriter.shared.send(
                    Event(event: "progress",
                        fraction: Double(progress.step) / Double(max(1, progress.stepCount)),
                        step: progress.step, totalSteps: progress.stepCount))
                return true
            }
            cg = images.compactMap { $0 }.first
        }
        guard let image = cg, let b64 = pngBase64(image) else {
            LineWriter.shared.send(Response(op: "generate", ok: false, code: 5, error: "generation produced no image"))
            return
        }
        LineWriter.shared.send(Response(op: "generate", ok: true, imageBase64: b64))
    } catch {
        LineWriter.shared.send(Response(op: "generate", ok: false, code: 5, error: "\(error)"))
    }
}

// MARK: - Dispatch

func handleCheck() {
    // Availability is "the binary launched" (macOS 14+); the engine loads
    // lazily at generate time. Engines list mirrors the catalog's `engine`s.
    let payload = "{\"available\":true,\"reason\":null,\"engines\":[\"sd\",\"sdxl\"]}"
    print(payload)
}

func handle(_ req: Request) async {
    switch req.op {
    case "installed":
        let dir = req.modelsDir ?? ""
        LineWriter.shared.send(Response(op: "installed", ok: true, installed: scanInstalled(modelsDir: dir)))
    case "download":
        guard let id = req.downloadId else {
            LineWriter.shared.send(Response(op: "download", ok: false, code: 4, error: "missing downloadId"))
            return
        }
        let task = Task { await runDownload(req) }
        await ActiveDownloads.shared.register(id, task)
    case "cancel-download":
        if let id = req.downloadId { await ActiveDownloads.shared.cancel(id) }
        LineWriter.shared.send(Response(op: "cancel-download", ok: true))
    case "delete":
        if let dir = req.modelsDir, let id = req.model {
            try? FileManager.default.removeItem(at: modelRoot(dir, id))
        }
        LineWriter.shared.send(Response(op: "delete", ok: true))
    case "generate":
        // Heavy ANE work runs on a detached task (off the serve loop's
        // cooperative thread), but we AWAIT it: the Rust client drops stdin
        // right after sending `generate`, so the next readLine returns EOF and
        // serve() would exit(0) — killing an un-awaited detached task before it
        // writes the image. Awaiting holds the process open until the terminal
        // response (and all progress events) have been emitted.
        await Task.detached { runDiffusion(req) }.value
    default:
        LineWriter.shared.send(Response(op: req.op, ok: false, code: 4, error: "unknown op"))
    }
}

func serve() async {
    let decoder = JSONDecoder()
    while let line = readLine(strippingNewline: true) {
        if line.isEmpty { continue }
        guard let data = line.data(using: .utf8), let req = try? decoder.decode(Request.self, from: data)
        else {
            LineWriter.shared.send(Response(op: "?", ok: false, code: 4, error: "malformed request"))
            continue
        }
        await handle(req)
    }
}

// MARK: - Entry

let args = CommandLine.arguments
if args.contains("--check") {
    handleCheck()
    exit(0)
} else if args.contains("--serve") {
    // Keep the process alive for detached generate/download tasks: the serve
    // loop awaits stdin; tasks write through LineWriter as they progress.
    let sema = DispatchSemaphore(value: 0)
    Task {
        await serve()
        sema.signal()
    }
    sema.wait()
    exit(0)
} else {
    FileHandle.standardError.write(Data("usage: portbay-imagegen --check | --serve\n".utf8))
    exit(2)
}
