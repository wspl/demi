// A foldable simulator's hinge, as Xcode's Device Hub moves it.
//
// The angle reaches the guest as a vendor-defined HID event to dtuhidd:
// `IndigoVendorDefinedEvent` for the feature
// `com.apple.coredevice.feature.remote.hid.vendordefined`, over the XPC
// endpoint the device answers for its digitizer's service, with usage page
// 0xFF61, usage 0x5B, version 0 and IOKit's binary serialization of
// `{provider: com.apple.Virtualization.VirtualMachines, source:
// hinge-slider-control, type: range, value: <degrees>}`, the angle a real
// number, as Device Hub sends it. The guest's CoreMotion turns it into hinge
// events, SpringBoard into a posture and a lit display.
//
// The hinge moves as Device Hub moves it: an angle every 16 ms on a
// critically damped spring towards its target, which a new target turns from
// where the hinge is, at the speed it has, so apps see the fold move and a
// drag never queues.

import FBSimulatorControl
import Foundation
import IOKit
import QuartzCore
import XPC

/// A hinge's angle laid flat.
let FLAT_DEGREES = 180

final class Hinge: @unchecked Sendable {
  private static let service = "com.apple.coredevice.feature.remote.hid.digitizer"
  private static let feature = "com.apple.coredevice.feature.remote.hid.vendordefined"
  /// The time between two angles, Device Hub's.
  static let frameNanoseconds: UInt64 = 16_000_000
  /// The spring's smoothing time, Device Hub's: about the time it takes to
  /// cover most of the way.
  static let smoothing = 0.2
  /// How near the target, in degrees, and how slow, in degrees a second,
  /// the hinge is when it arrives.
  static let arrivedWithin = 0.25
  static let arrivedBelow = 3.0

  /// The display the guest lights after the hinge moved to `degrees` from
  /// `from` with `lit` lit, as measured on iPhone Duo with iOS 27.1: moved
  /// in one go, past 90° lights the inner display and below 20° the outer
  /// one; moved by less than 20°, as a slider drags it, the outer display
  /// stays lit up to 120°. In between, the lit display stays.
  static func lit(after degrees: Int, from: Int, lit: FoldDisplay) -> FoldDisplay {
    if degrees < 20 { return .outer }
    let innerAbove = abs(degrees - from) < 20 ? 120 : 90
    return degrees > innerAbove ? .inner : lit
  }

  private let connection: xpc_connection_t
  private let lock = NSLock()
  /// The angle sent last, and the speed it moved at, in degrees a second.
  private var angle: Double
  private var velocity = 0.0
  /// Where the hinge moves to; none while it rests.
  private var target: Double?
  /// Those waiting for the hinge to come to rest.
  private var waiters: [CheckedContinuation<Void, Never>] = []

  private init(connection: xpc_connection_t, angle: Int) {
    self.connection = connection
    self.angle = Double(angle)
  }

  /// Opens the hinge of `simulator`, which must be booted, resting at
  /// `angle`.
  static func open(_ simulator: Simulator, at angle: Int) throws -> Hinge {
    let device = simulator.device as AnyObject
    let lookup = NSSelectorFromString("lookup:error:")
    guard device.responds(to: lookup) else {
      throw HelperError("this Xcode's CoreSimulator cannot reach a device's services")
    }
    typealias Lookup = @convention(c) (AnyObject, Selector, NSString, UnsafeMutablePointer<NSError?>?) -> mach_port_t
    let call = unsafeBitCast(device.method(for: lookup), to: Lookup.self)
    var error: NSError?
    let port = call(device, lookup, service as NSString, &error)
    guard port != MACH_PORT_NULL else {
      throw HelperError("\(simulator.name)'s hinge did not answer: \(error.map(describe) ?? "no port")")
    }
    typealias Endpoint = @convention(c) (mach_port_t, Int32, Int32) -> Unmanaged<xpc_endpoint_t>?
    typealias Enable = @convention(c) (xpc_connection_t) -> Void
    guard let endpointSymbol = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "xpc_endpoint_create_mach_port_4sim"),
      let enableSymbol = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "xpc_connection_enable_sim2host_4sim"),
      let endpoint = unsafeBitCast(endpointSymbol, to: Endpoint.self)(port, 0, 0)?.takeRetainedValue()
    else {
      throw HelperError("this macOS cannot connect to a simulator's services")
    }
    let connection = xpc_connection_create_from_endpoint(endpoint)
    unsafeBitCast(enableSymbol, to: Enable.self)(connection)
    xpc_connection_set_event_handler(connection) { event in
      if xpc_get_type(event) == XPC_TYPE_ERROR, event !== XPC_ERROR_CONNECTION_INVALID {
        let description = xpc_dictionary_get_string(event, XPC_ERROR_KEY_DESCRIPTION).map { String(cString: $0) }
        log("\(simulator.name)'s hinge: \(description ?? "an error")")
      }
    }
    xpc_connection_resume(connection)
    return Hinge(connection: connection, angle: angle)
  }

  /// Whether the hinge moves now.
  var moving: Bool {
    lock.withLock { target != nil }
  }

  /// Moves the hinge towards `degrees` at once: a move under way turns
  /// towards it from where it is, and a hinge at rest starts from `from`, the
  /// angle the device has, which a fold outside Demi may have changed.
  func move(to degrees: Int, from: Int) {
    let start = lock.withLock { () -> Bool in
      let idle = target == nil
      if idle {
        angle = Double(from)
        velocity = 0
      }
      target = Double(min(max(degrees, 0), FLAT_DEGREES))
      return idle
    }
    if start {
      Task { await run() }
    }
  }

  /// Returns once the hinge rests, wherever the last move took it, and the
  /// guest has every angle.
  func rest() async {
    await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
      let resting = lock.withLock { () -> Bool in
        if target == nil { return true }
        waiters.append(continuation)
        return false
      }
      if resting { continuation.resume() }
    }
  }

  /// Sends an angle every frame until the hinge arrives at its target.
  private func run() async {
    var last = CACurrentMediaTime()
    while true {
      let now = CACurrentMediaTime()
      // A frame late by more than 50 ms moves as one of 50 ms, so a stall
      // never throws the hinge past its target.
      let elapsed = min(max(now - last, 0), 0.05)
      last = now
      let (next, arrived) = lock.withLock { () -> (Double, Bool) in
        guard let target else { return (angle, true) }
        (angle, velocity) = Hinge.spring(from: angle, velocity: velocity, to: target, after: elapsed)
        let arrived = abs(angle - target) < Hinge.arrivedWithin && abs(velocity) < Hinge.arrivedBelow
        if arrived {
          angle = target
          velocity = 0
        }
        return (angle, arrived)
      }
      send(next)
      if arrived { break }
      try? await Task.sleep(nanoseconds: Hinge.frameNanoseconds)
    }
    await withCheckedContinuation { (done: CheckedContinuation<Void, Never>) in
      xpc_connection_send_barrier(connection) { done.resume() }
    }
    // A move that came during the barrier goes on from here.
    let (waiting, again) = lock.withLock { () -> ([CheckedContinuation<Void, Never>], Bool) in
      if let target, abs(angle - target) >= Hinge.arrivedWithin { return ([], true) }
      target = nil
      defer { waiters.removeAll() }
      return (waiters, false)
    }
    if again {
      await run()
      return
    }
    for waiter in waiting { waiter.resume() }
  }

  /// One step of a critically damped spring of `smoothing` from `angle`
  /// moving at `velocity` towards `target`, as Device Hub steps it, never
  /// past the target.
  static func spring(from angle: Double, velocity: Double, to target: Double, after elapsed: Double)
    -> (Double, Double)
  {
    let omega = 2 / smoothing
    let x = omega * elapsed
    let decay = 1 / (1 + x + 0.48 * x * x + 0.235 * x * x * x)
    let offset = angle - target
    let pull = (velocity + omega * offset) * elapsed
    var velocity = (velocity - omega * pull) * decay
    var next = target + (offset + pull) * decay
    if (target - angle > 0) == (next > target) {
      next = target
      velocity = 0
    }
    return (next, velocity)
  }

  private func send(_ degrees: Double) {
    let event: NSDictionary = [
      "provider": "com.apple.Virtualization.VirtualMachines", "source": "hinge-slider-control",
      "type": "range", "value": NSNumber(value: degrees),
    ]
    guard let data = IOCFSerialize(event, CFOptionFlags(kIOCFSerializeToBinary)) as Data? else { return }
    let payload = xpc_dictionary_create(nil, nil, 0)
    xpc_dictionary_set_uint64(payload, "usagePage", 0xFF61)
    xpc_dictionary_set_uint64(payload, "usage", 0x5B)
    xpc_dictionary_set_uint64(payload, "version", 0)
    data.withUnsafeBytes { xpc_dictionary_set_data(payload, "data", $0.baseAddress!, data.count) }
    let message = xpc_dictionary_create(nil, nil, 0)
    xpc_dictionary_set_string(message, "messageType", "IndigoVendorDefinedEvent")
    xpc_dictionary_set_bool(message, "isBarrier", false)
    xpc_dictionary_set_string(message, "featureIdentifier", Hinge.feature)
    xpc_dictionary_set_value(message, "payload", payload)
    xpc_connection_send_message(connection, message)
  }

  func close() {
    xpc_connection_cancel(connection)
  }
}
