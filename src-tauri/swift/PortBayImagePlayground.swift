// PortBayImagePlayground — in-process Apple Image Playground bridge.
//
// Apple's `ImageCreator` (ImagePlayground framework) generates images
// programmatically, but throws `backgroundCreationForbidden` unless the calling
// process is the frontmost, active GUI app. PortBay is exactly that, so — unlike
// the headless Stable Diffusion sidecar — this runs IN the app process: a tiny
// Swift static library (compiled + linked by build.rs on macOS) exposing two
// C-callable entry points the Rust `imageplayground` module calls.
//
// Both are synchronous wrappers (a semaphore drives Swift concurrency to
// completion) returning a malloc'd, NUL-terminated C string the caller frees via
// `portbay_ip_free`:
//   portbay_ip_check()           -> "ok" | "requires_macos_15_4" |
//                                   "apple_intelligence_unavailable" |
//                                   "unsupported_device" | "unavailable"
//   portbay_ip_generate(prompt)  -> "OK:<base64-png>" | "CANCEL" | "NOTREADY" |
//                                   "ERR:<msg>"
//
// Call `portbay_ip_generate` off the main thread (it blocks until generation
// finishes); the app's run loop keeps pumping so AppKit stays responsive.

import AppKit
import Foundation
// ImagePlayground only ships in the macOS 15.4+ SDK. Toolchains that predate it
// (e.g. the public-CI runners) can't import the module at all, so the whole
// bridge is compiled conditionally — `#if canImport` — and falls back to stub
// entry points that report it unavailable, keeping the C ABI the Rust
// `imageplayground` module links against intact on every toolchain.
#if canImport(ImagePlayground)
import ImagePlayground
#endif

private func cstr(_ s: String) -> UnsafeMutablePointer<CChar>? { strdup(s) }

@_cdecl("portbay_ip_free")
public func portbay_ip_free(_ p: UnsafeMutablePointer<CChar>?) {
    free(p)
}

#if canImport(ImagePlayground)

@_cdecl("portbay_ip_check")
public func portbay_ip_check() -> UnsafeMutablePointer<CChar>? {
    guard #available(macOS 15.4, *) else { return cstr("requires_macos_15_4") }
    let sema = DispatchSemaphore(value: 0)
    var reason = "ok"
    Task {
        defer { sema.signal() }
        do {
            _ = try await ImageCreator()
        } catch let error as ImageCreator.Error {
            switch error {
            case .notSupported: reason = "unsupported_device"
            case .unavailable: reason = "apple_intelligence_unavailable"
            default: reason = "unavailable"
            }
        } catch {
            reason = "unavailable"
        }
    }
    sema.wait()
    return cstr(reason)
}

@_cdecl("portbay_ip_generate")
public func portbay_ip_generate(_ promptC: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>? {
    guard #available(macOS 15.4, *) else { return cstr("ERR:requires macOS 15.4") }
    let prompt = promptC.map { String(cString: $0) } ?? ""
    let sema = DispatchSemaphore(value: 0)
    var out = "CANCEL"
    // Run generation on the main actor. ImageCreator rejects requests that
    // don't originate from the app's foreground UI context with
    // `backgroundCreationForbidden` — driving it from a background-QoS thread
    // (which is where this FFI call lands, off a tokio blocking pool) trips
    // that even when PortBay is frontmost. The `await` suspension points let
    // the run loop keep pumping, so AppKit stays responsive; the semaphore is
    // waited on the *calling* (background) thread, so there's no main-thread
    // deadlock.
    var diag = ""
    Task { @MainActor in
        defer { sema.signal() }
        do {
            // The user pressed Generate in PortBay — make sure we're frontmost
            // and active, which ImageCreator requires for the whole run.
            NSApplication.shared.activate(ignoringOtherApps: true)
            // Snapshot what the system sees, so a foreground rejection is
            // explainable instead of a guess. Embedded in the error below.
            let app = NSApplication.shared
            let front = NSWorkspace.shared.frontmostApplication?.bundleIdentifier ?? "nil"
            let hasVisibleWindow = app.windows.contains { $0.isVisible }
            diag = "active=\(app.isActive) hidden=\(app.isHidden) "
                + "policy=\(app.activationPolicy().rawValue) keyWindow=\(app.keyWindow != nil) "
                + "visibleWindow=\(hasVisibleWindow) windows=\(app.windows.count) "
                + "frontmost=\(front)"
            let creator = try await ImageCreator()
            guard let style = creator.availableStyles.first else {
                out = "ERR:no image styles available"
                return
            }
            let concepts: [ImagePlaygroundConcept] = prompt.isEmpty ? [] : [.text(prompt)]
            let seq = creator.images(for: concepts, style: style, limit: 1)
            for try await created in seq {
                let rep = NSBitmapImageRep(cgImage: created.cgImage)
                if let png = rep.representation(using: .png, properties: [:]) {
                    out = "OK:" + png.base64EncodedString()
                } else {
                    out = "ERR:could not encode the generated image"
                }
                break
            }
        } catch let error as ImageCreator.Error {
            switch error {
            case .creationCancelled: out = "CANCEL"
            case .backgroundCreationForbidden:
                out = "ERR:Image Playground refused: app considered backgrounded. [\(diag)]"
            // `notSupported` at generate time, after init succeeded above, means
            // the system's image-model resources aren't downloaded — a genuinely
            // unsupported device already fails ImageCreator() in the startup
            // check. (Verified live: GPNonUIExtension logs "Image Generation is
            // unavailable: Spotlight resources not downloaded" for this case.)
            case .notSupported: out = "NOTREADY"
            case .unavailable: out = "ERR:turn on Apple Intelligence to use Image Playground"
            case .unsupportedLanguage: out = "ERR:that prompt's language isn't supported yet"
            case .creationFailed: out = "ERR:Image Playground couldn't create an image for that prompt"
            default: out = "ERR:\(error)"
            }
        } catch {
            out = "ERR:\(error)"
        }
    }
    sema.wait()
    return cstr(out)
}

#else

// Built without the ImagePlayground SDK module. The symbols still have to exist
// (Rust links against them), but the feature is genuinely unavailable on this
// toolchain — report it the same way the runtime `#available` guard does.

@_cdecl("portbay_ip_check")
public func portbay_ip_check() -> UnsafeMutablePointer<CChar>? {
    return cstr("requires_macos_15_4")
}

@_cdecl("portbay_ip_generate")
public func portbay_ip_generate(_ promptC: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>? {
    return cstr("ERR:requires macOS 15.4")
}

#endif
