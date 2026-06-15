// portbay-afm — Apple Foundation Models bridge for Smart Dictation.
//
// Why a sidecar: the FoundationModels framework (macOS 26's on-device LLM,
// the "Apple Intelligence" model) exposes a Swift-only API — there is no
// ObjC surface the Rust app could reach through objc2. This tiny CLI is the
// bridge. Two transports: the app normally keeps one `--serve` process warm
// (line-delimited JSON requests/responses — saves process start + framework
// load per rewrite) and falls back to the stateless one-shot mode (one JSON
// request on stdin, rewritten text on stdout) when the server is unusable.
// The model itself is resident system-wide either way — the OS inference
// daemon runs the actual generation.
//
// Protocol:
//   portbay-afm --check
//     → stdout: one-line JSON {"available":Bool,"reason":String?}; exit 0.
//       `reason` is machine-readable for the settings UI: requires_macos_26 |
//       device_not_eligible | apple_intelligence_not_enabled |
//       model_not_ready | unavailable.
//   portbay-afm --prewarm
//     ← stdin (optional): the static instructions head the next rewrite will
//       use, so the OS can pre-process the system prompt, not just page the
//       model in.
//     → best-effort hint to page the OS model in (fired at dictation START
//       so the rewrite at dictation end doesn't pay first-use load); always
//       exit 0, no output.
//   portbay-afm --serve   (warm-server mode)
//     ← stdin:  one JSON request per line {"system":String,"prompt":String,
//       "maxTokens":Int} — same shape as the one-shot mode.
//     → stdout: one JSON response per line {"ok":true,"text":String} or
//       {"ok":false,"code":Int,"error":String}; `code` mirrors the one-shot
//       exit codes below. Serial — one request in flight at a time. EOF on
//       stdin = shutdown (exit 0). The win over one-shot: process start +
//       framework load are paid once per dictation session, not per rewrite.
//   portbay-afm   (default, one-shot mode)
//     ← stdin:  JSON {"system":String,"prompt":String,"maxTokens":Int}
//     → stdout: the model's text response; exit 0.
//   Exit codes: 2 model unavailable · 3 generation refused (guardrails) ·
//   4 bad request · 5 generation failed. Stderr carries the detail; the app
//   maps every non-zero exit to "keep the raw transcript".
//
// Built by scripts/build-afm.sh (deployment target macOS 13 — on anything
// older the exec itself fails, which the app treats the same as any other
// rewrite failure: the dictated text stays as spoken).

import Foundation
#if canImport(FoundationModels)
import FoundationModels
#endif

struct Request: Decodable {
    let system: String
    let prompt: String
    let maxTokens: Int
}

func die(_ code: Int32, _ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(code)
}

/// Availability + machine-readable reason, resolved once per invocation.
func availability() -> (available: Bool, reason: String?) {
    guard #available(macOS 26.0, *) else { return (false, "requires_macos_26") }
    #if canImport(FoundationModels)
    switch SystemLanguageModel.default.availability {
    case .available:
        return (true, nil)
    case .unavailable(.deviceNotEligible):
        return (false, "device_not_eligible")
    case .unavailable(.appleIntelligenceNotEnabled):
        return (false, "apple_intelligence_not_enabled")
    case .unavailable(.modelNotReady):
        return (false, "model_not_ready")
    case .unavailable:
        return (false, "unavailable")
    }
    #else
    // Compiled against a pre-26 SDK: the framework cannot exist at runtime
    // for this binary even on macOS 26.
    return (false, "built_without_sdk")
    #endif
}

// Sidecar↔host wire-protocol version, kept in lockstep with
// src-tauri/src/sidecar_protocol.rs::SIDECAR_PROTOCOL. The release build's
// scripts/verify-sidecars.sh runs `--protocol` on every freshly built sidecar
// and fails if any diverges, so a stale or mismatched binary can't ship. Bump
// together when the stdin/stdout JSON protocol changes.
if CommandLine.arguments.contains("--protocol") {
    print("1")
    exit(0)
}

if CommandLine.arguments.contains("--check") {
    let (_, reason) = availability()
    if let reason {
        print("{\"available\":false,\"reason\":\"\(reason)\"}")
    } else {
        print("{\"available\":true}")
    }
    exit(0)
}

if CommandLine.arguments.contains("--prewarm") {
    // Best-effort: ask the system inference daemon to page the model in now
    // (Apple's guidance is to prewarm when a request is *anticipated* — the
    // app fires this at dictation start, the rewrite comes at dictation
    // end). Inference runs in an OS daemon, not this process, but give the
    // hint a moment to be delivered before exiting. Never fails: a machine
    // that can't prewarm pays first-use load like before, nothing worse.
    //
    // stdin may carry the static instructions head the next rewrite will use
    // (BASE_RULES + examples — see dictation.rs::prompt_head). Prewarming a
    // session that already holds those instructions lets the OS pre-process
    // the system prompt, not just page model weights in; an empty stdin
    // degrades to the old weights-only hint.
    #if canImport(FoundationModels)
    if #available(macOS 26.0, *), case .available = SystemLanguageModel.default.availability {
        let head = String(
            data: FileHandle.standardInput.readDataToEndOfFile(),
            encoding: .utf8
        )?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let model = SystemLanguageModel(guardrails: .permissiveContentTransformations)
        let session = head.isEmpty
            ? LanguageModelSession(model: model)
            : LanguageModelSession(model: model, instructions: head)
        // The session carries the static instructions head; the prompt
        // prefix is the framing every rewrite's user message starts with
        // (dictation.rs::build_user) — together they cover everything
        // static about the next request.
        session.prewarm(promptPrefix: Prompt("Transcript:"))
        try? await Task.sleep(nanoseconds: 2_000_000_000)
    }
    #endif
    exit(0)
}

// ---------------------------------------------------------------------------
// Warm-server mode (--serve)
// ---------------------------------------------------------------------------

/// One-line JSON response for serve mode. JSONEncoder escapes newlines inside
/// strings, so a response is always exactly one stdout line.
struct ServeResponse: Encodable {
    let ok: Bool
    var text: String? = nil
    var code: Int? = nil
    var error: String? = nil
}

func writeServeLine(_ response: ServeResponse) {
    let data = (try? JSONEncoder().encode(response))
        ?? Data("{\"ok\":false,\"code\":5,\"error\":\"response encode failed\"}".utf8)
    // FileHandle writes are unbuffered syscalls — no flush dance needed for
    // the line-delimited protocol.
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data("\n".utf8))
}

/// Run one rewrite for serve mode. A fresh `LanguageModelSession` per request
/// on purpose: `respond` appends to the session transcript, so reusing one
/// would leak earlier rewrites into later context. The warm win is the
/// process + framework load, which this loop pays once; the model itself
/// lives in the OS inference daemon either way.
@available(macOS 26.0, *)
func serveOne(_ request: Request) async -> ServeResponse {
    #if canImport(FoundationModels)
    guard case .available = SystemLanguageModel.default.availability else {
        return ServeResponse(ok: false, code: 2, error: "on-device model unavailable")
    }
    // Same configuration as the one-shot path — see runRewrite for why
    // permissive guardrails + greedy sampling.
    let model = SystemLanguageModel(guardrails: .permissiveContentTransformations)
    let session = LanguageModelSession(model: model, instructions: request.system)
    let options = GenerationOptions(sampling: .greedy, maximumResponseTokens: request.maxTokens)
    do {
        let response = try await session.respond(to: request.prompt, options: options)
        return ServeResponse(ok: true, text: response.content)
    } catch let error as LanguageModelSession.GenerationError {
        switch error {
        case .guardrailViolation, .refusal, .unsupportedLanguageOrLocale:
            return ServeResponse(ok: false, code: 3, error: "generation refused: \(error.localizedDescription)")
        case .assetsUnavailable, .rateLimited, .concurrentRequests:
            return ServeResponse(ok: false, code: 2, error: "model temporarily unavailable: \(error.localizedDescription)")
        default:
            return ServeResponse(ok: false, code: 5, error: "generation failed: \(error.localizedDescription)")
        }
    } catch {
        return ServeResponse(ok: false, code: 5, error: "generation failed: \(error.localizedDescription)")
    }
    #else
    return ServeResponse(ok: false, code: 2, error: "built without the FoundationModels SDK")
    #endif
}

if CommandLine.arguments.contains("--serve") {
    // Serial request loop: read a line, answer a line. A malformed line gets
    // an error response and the loop continues — only stdin EOF (the app
    // closed the pipe or died) ends the server. Availability is re-checked
    // per request, so Apple Intelligence toggling mid-session degrades to
    // per-request code-2 errors rather than a dead process.
    while let line = readLine(strippingNewline: true) {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        if trimmed.isEmpty { continue }
        guard let request = try? JSONDecoder().decode(Request.self, from: Data(trimmed.utf8)) else {
            writeServeLine(ServeResponse(ok: false, code: 4, error: "invalid rewrite request"))
            continue
        }
        if #available(macOS 26.0, *) {
            writeServeLine(await serveOne(request))
        } else {
            writeServeLine(ServeResponse(ok: false, code: 2, error: "requires macOS 26"))
        }
    }
    exit(0)
}

// Guided generation (@Generable single-String schema) was TRIED AND
// REVERTED here (2026-06-06, probed offline against the tuned prompt) —
// don't re-add it without re-probing:
//   • with the schema in the prompt it dilutes the load-bearing system
//     prompt: spoken self-corrections regressed AND one instruction-shaped
//     transcript was answered with a fully HALLUCINATED paragraph that
//     passed output validation (no boilerplate prefix, within growth cap);
//   • with includeSchemaInPrompt:false, constrained decoding suppresses
//     multi-line output — the enumeration→numbered-list behavior (an
//     explicit product requirement) disappears, and @Guide text can't
//     steer it back because the model never sees the schema.
// Plain-text generation + the app-side sanitizer keeps every failure mode
// at "keep the raw transcript"; guided generation's failure mode is clean-
// looking invented text. Format noise (fences/labels/preamble) is already
// handled post-hoc in dictation.rs::sanitize_output.

/// Run one rewrite and exit. A function (not top-level code) because
/// availability scoping from a top-level `guard #available` does not extend
/// to the following top-level statements.
@available(macOS 26.0, *)
func runRewrite(_ request: Request) async -> Never {
    #if canImport(FoundationModels)
    guard case .available = SystemLanguageModel.default.availability else {
        die(2, "portbay-afm: on-device model unavailable")
    }

    // Permissive-transformation guardrails: this sidecar only ever rewrites
    // text the user themselves dictated, which is exactly the use case Apple
    // ships these guardrails for. The default guardrails refuse casual
    // profanity in the user's own speech ("Detected content likely to be
    // unsafe", observed live 2026-06-06), nuking legitimate rewrites/edits.
    let model = SystemLanguageModel(guardrails: .permissiveContentTransformations)
    let session = LanguageModelSession(model: model, instructions: request.system)
    // Greedy, bounded — rewriting is transformation, not generation: the
    // most-likely token is the right token, and determinism beats variety.
    let options = GenerationOptions(sampling: .greedy, maximumResponseTokens: request.maxTokens)
    do {
        let response = try await session.respond(to: request.prompt, options: options)
        print(response.content)
        exit(0)
    } catch let error as LanguageModelSession.GenerationError {
        switch error {
        case .guardrailViolation, .refusal, .unsupportedLanguageOrLocale:
            // The model declined this input — safety layer, refusal, or a
            // dictation language the on-device model doesn't support. A
            // normal outcome (the raw text stays in the field), not an app
            // error; the next English transcript will rewrite fine.
            die(3, "portbay-afm: generation refused: \(error.localizedDescription)")
        case .assetsUnavailable, .rateLimited, .concurrentRequests:
            // Transient model-side unavailability — same handling as "model
            // not ready": provider error, raw transcript kept.
            die(2, "portbay-afm: model temporarily unavailable: \(error.localizedDescription)")
        default:
            // exceededContextWindowSize / decodingFailure / unsupported* —
            // unexpected for short dictation transcripts; log-worthy detail.
            die(5, "portbay-afm: generation failed: \(error.localizedDescription)")
        }
    } catch {
        die(5, "portbay-afm: generation failed: \(error.localizedDescription)")
    }
    #else
    die(2, "portbay-afm: built without the FoundationModels SDK")
    #endif
}

let input = FileHandle.standardInput.readDataToEndOfFile()
guard let request = try? JSONDecoder().decode(Request.self, from: input) else {
    die(4, "portbay-afm: stdin is not a valid rewrite request")
}

if #available(macOS 26.0, *) {
    await runRewrite(request)
} else {
    die(2, "portbay-afm: requires macOS 26")
}
