// `serve`: one conversation's helper. Each request names its method and
// its device; replies carry the request's id, a file before its reply in a
// blob frame. Requests on one device run in turn, requests on different
// devices at once.

import FBControlCore
import FBSimulatorControl
import Foundation

final class Server: @unchecked Sendable {
  let output = Output()
  private let lock = NSLock()
  private var devices: [String: DeviceSession] = [:]
  /// Each device's requests, in order.
  private var queues: [String: SerialQueue] = [:]

  func run() async throws {
    // Standard input is read on a thread of its own, since FileHandle
    // blocks.
    let frames = AsyncStream<(FrameKind, Data)> { continuation in
      Thread.detachNewThread {
        var input = Input()
        while let frame = input.next() { continuation.yield(frame) }
        continuation.finish()
      }
    }
    for await (kind, payload) in frames {
      guard kind == .control,
        let request = try? JSONSerialization.jsonObject(with: payload) as? [String: Any],
        let id = (request["id"] as? NSNumber)?.uint32Value,
        let method = request["method"] as? String
      else {
        log("a request did not read")
        continue
      }
      let params = request["params"] as? [String: Any] ?? [:]
      let udid = params["udid"] as? String ?? ""
      // A capture's control does not wait for a slow read of the tree.
      let lane = method.hasPrefix("capture") ? "\(udid)/capture" : udid
      queue(lane).run { [self] in
        do {
          let (result, blob) = try await handle(method, udid: udid, params: params)
          if let blob { output.blob(id: id, blob) }
          output.control(["id": id, "result": result])
        } catch {
          output.control(["id": id, "error": describe(error)])
        }
      }
    }
    // demi-mobile closed its end: every device lets go as detach does.
    for session in lock.withLock({ Array(devices.values) }) {
      await session.detach()
    }
  }

  private func queue(_ udid: String) -> SerialQueue {
    lock.withLock {
      if let queue = queues[udid] { return queue }
      let queue = SerialQueue()
      queues[udid] = queue
      return queue
    }
  }

  private func session(_ udid: String) throws -> DeviceSession {
    guard let session = lock.withLock({ devices[udid] }) else {
      throw HelperError("the simulator \(udid) is not attached")
    }
    return session
  }

  private func handle(_ method: String, udid: String, params: [String: Any]) async throws
    -> ([String: Any], Data?)
  {
    if method == "attach" {
      let session = try await DeviceSession.attach(udid: udid, output: output)
      lock.withLock { devices[udid] = session }
      return (session.screenJSON(), nil)
    }
    let session = try session(udid)
    switch method {
    case "detach":
      lock.withLock { devices[udid] = nil }
      await session.detach()
      return ([:], nil)
    case "screenshot":
      let longSide = (params["longSide"] as? NSNumber)?.intValue ?? 1568
      let (png, width, height) = try session.screenshot(longSide: longSide)
      return (["width": width, "height": height], png)
    case "tree":
      return (["elements": try await session.tree()], nil)
    case "touch":
      try await session.touch(
        phase: params["phase"] as? String ?? "", x: double(params["x"]), y: double(params["y"]))
      return ([:], nil)
    case "tap":
      try await session.tap(x: double(params["x"]), y: double(params["y"]), seconds: double(params["seconds"]))
      return ([:], nil)
    case "swipe":
      try await session.swipe(
        from: CGPoint(x: double(params["x1"]), y: double(params["y1"])),
        to: CGPoint(x: double(params["x2"]), y: double(params["y2"])))
      return ([:], nil)
    case "button":
      try await session.button(params["button"] as? String ?? "")
      return ([:], nil)
    case "key":
      try await session.key(
        usage: UInt32((params["usage"] as? NSNumber)?.uint32Value ?? 0),
        down: params["down"] as? Bool ?? false)
      return ([:], nil)
    case "type":
      let keys = (params["keys"] as? [[Any]] ?? []).map { pair in
        (UInt32((pair.first as? NSNumber)?.uint32Value ?? 0), (pair.last as? Bool) ?? false)
      }
      try await session.type(keys)
      return ([:], nil)
    case "setValue":
      try await session.setValue(
        params["text"] as? String ?? "", x: double(params["x"]), y: double(params["y"]))
      return ([:], nil)
    case "orient":
      try await session.orient(params["orientation"] as? String ?? "")
      return (session.screenJSON(), nil)
    case "appearance":
      return (["appearance": try await session.appearance()], nil)
    case "setAppearance":
      try await session.setAppearance(params["appearance"] as? String ?? "")
      return ([:], nil)
    case "recordStart":
      guard let path = params["path"] as? String else { throw HelperError("recordStart names no path") }
      try session.recordStart(path: path)
      return ([:], nil)
    case "recordStop":
      return (["durationMs": try await session.recordStop()], nil)
    case "frame":
      return try session.frame(
        area: CGSize(width: double(params["width"]), height: double(params["height"])), backing: double(params["scale"]))
    case "captureStart":
      try session.captureStart(
        capture: UInt32((params["capture"] as? NSNumber)?.uint32Value ?? 0),
        width: Int(double(params["width"])), height: Int(double(params["height"])),
        fps: Int(double(params["fps"])), bitrate: Int(double(params["bitrate"])))
      return ([:], nil)
    case "captureKeyFrame":
      session.captureKeyFrame(capture: UInt32((params["capture"] as? NSNumber)?.uint32Value ?? 0))
      return ([:], nil)
    case "captureAck":
      session.captureAck(
        capture: UInt32((params["capture"] as? NSNumber)?.uint32Value ?? 0),
        sequence: (params["sequence"] as? NSNumber)?.uint32Value ?? 0,
        window: (params["window"] as? NSNumber)?.uint32Value ?? 0,
        order: (params["order"] as? NSNumber)?.uint64Value ?? 0)
      return ([:], nil)
    case "captureEncoding":
      session.captureEncoding(
        capture: UInt32((params["capture"] as? NSNumber)?.uint32Value ?? 0),
        bitrate: Int(double(params["bitrate"])), fps: Int(double(params["fps"])))
      return ([:], nil)
    case "captureStop":
      session.captureStop(capture: UInt32((params["capture"] as? NSNumber)?.uint32Value ?? 0))
      return ([:], nil)
    default:
      throw HelperError("the iOS helper has no method \(method)")
    }
  }

  private func double(_ value: Any?) -> Double {
    (value as? NSNumber)?.doubleValue ?? 0
  }
}

/// Runs its jobs one after another, each to its end.
final class SerialQueue: @unchecked Sendable {
  private let lock = NSLock()
  private var last: Task<Void, Never>?

  func run(_ job: @escaping @Sendable () async -> Void) {
    lock.withLock {
      let previous = last
      last = Task {
        await previous?.value
        await job()
      }
    }
  }
}
