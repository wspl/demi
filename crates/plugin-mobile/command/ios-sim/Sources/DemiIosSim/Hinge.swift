// A foldable simulator's hinge, as Xcode's Device Hub moves it.
//
// The angle reaches the guest as a vendor-defined HID event to dtuhidd:
// `IndigoVendorDefinedEvent` for the feature
// `com.apple.coredevice.feature.remote.hid.vendordefined`, over the XPC
// endpoint the device answers for its digitizer's service, with usage page
// 0xFF61, usage 0x5B, version 1 and IOKit's compact XML of
// `{source: hinge-slider-control, type: range, value: <whole degrees>}`. The
// guest's CoreMotion turns it into hinge events, SpringBoard into a posture
// and a lit display. The guest reads whole degrees only, and takes a full
// plist as too large.

import FBSimulatorControl
import Foundation
import XPC

/// A hinge's angle laid flat.
let FLAT_DEGREES = 180

final class Hinge: @unchecked Sendable {
  private static let service = "com.apple.coredevice.feature.remote.hid.digitizer"
  private static let feature = "com.apple.coredevice.feature.remote.hid.vendordefined"
  /// A posture's sweep, as Device Hub sends it: about 35 angles 17 ms apart,
  /// easing out, so apps see the fold move rather than jump.
  static let sweepSteps = 35
  static let stepNanoseconds: UInt64 = 17_000_000

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

  private init(connection: xpc_connection_t) {
    self.connection = connection
  }

  /// Opens the hinge of `simulator`, which must be booted.
  static func open(_ simulator: Simulator) throws -> Hinge {
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
    return Hinge(connection: connection)
  }

  /// Moves the hinge from `from` to `to` degrees over a posture's sweep, and
  /// returns once the guest has every angle.
  func sweep(from: Int, to: Int) async {
    var last = from
    for step in 1...Hinge.sweepSteps {
      let linear = Double(step) / Double(Hinge.sweepSteps)
      let eased = 1 - pow(1 - linear, 3)
      let angle = Int((Double(from) + Double(to - from) * eased).rounded())
      if angle == last && step != Hinge.sweepSteps { continue }
      last = angle
      send(angle)
      try? await Task.sleep(nanoseconds: Hinge.stepNanoseconds)
    }
    await withCheckedContinuation { (done: CheckedContinuation<Void, Never>) in
      xpc_connection_send_barrier(connection) { done.resume() }
    }
  }

  private func send(_ degrees: Int) {
    let xml =
      "<dict><key>source</key><string>hinge-slider-control</string><key>type</key><string>range</string>"
      + "<key>value</key><integer>\(degrees)</integer></dict>"
    let payload = xpc_dictionary_create(nil, nil, 0)
    xpc_dictionary_set_uint64(payload, "usagePage", 0xFF61)
    xpc_dictionary_set_uint64(payload, "usage", 0x5B)
    xpc_dictionary_set_uint64(payload, "version", 1)
    let data = Array(xml.utf8)
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
