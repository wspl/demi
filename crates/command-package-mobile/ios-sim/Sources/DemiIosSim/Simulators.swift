// The Mac's simulators, from CoreSimulator's default device set, the one
// Xcode, `simctl` and Simulator.app use.

import FBControlCore
import FBSimulatorAX
import FBSimulatorControl
import Foundation

enum Simulators {
  /// A simulator as `list` prints it.
  struct Listed: Encodable {
    let udid: String
    let name: String
    let model: String
    let os: String
    let state: String
  }

  /// The default device set, loaded once a process.
  static let control: Result<SimulatorControlBootstrap, Error> = Result {
    try SimulatorControlBootstrap(configuration: .init(deviceSetPath: nil, logger: nil))
  }

  static func set() throws -> SimulatorSet {
    try control.get().set
  }

  /// The iOS simulators, the newest system's first.
  static func list() throws -> [Listed] {
    try set().allSimulators
      .filter { $0.osVersion.name.rawValue.hasPrefix("iOS") }
      .map { simulator in
        Listed(
          udid: simulator.udid,
          name: simulator.name,
          model: simulator.deviceType.model.rawValue,
          os: simulator.osVersion.name.rawValue,
          state: stateName(simulator.state)
        )
      }
  }

  /// The state as demi-mobile reads it, which `simctl list` words alike.
  static func stateName(_ state: TargetState) -> String {
    switch state {
    case .booted: return "Booted"
    case .booting: return "Booting"
    case .shuttingDown: return "Shutting Down"
    default: return "Shutdown"
    }
  }

  static func named(_ udid: String) throws -> Simulator {
    guard let simulator = try set().simulator(withUDID: udid) else {
      throw HelperError("no simulator has the simctl id \(udid)")
    }
    return simulator
  }

  /// How long a booted simulator may take to show its home screen.
  static let homeScreenLimit: Duration = .seconds(60)

  /// Boots the simulator without Simulator.app and waits until it is usable:
  /// idb's own wait ends before SpringBoard is the frontmost application,
  /// and `inspect` reads that application, so the boot also waits until the
  /// accessibility service names one. The simulator announces neither, so
  /// both are waits that try again, as idb's `verifyUsable` is.
  static func boot(_ udid: String) async throws {
    let simulator = try named(udid)
    try await simulator.lifecycle.boot(.default)
    let automation = try simulator.uiAutomation(backend: .accessibility)
    let limit = ContinuousClock.now + homeScreenLimit
    while true {
      do {
        _ = try await automation.describe(.frontmost, options: AccessibilityRequestOptions(format: .default, keys: []))
        return
      } catch where ContinuousClock.now < limit {
        try await Task.sleep(for: .milliseconds(250))
      }
    }
  }

  static func shutdown(_ udid: String) async throws {
    try await named(udid).power.shutdown()
  }
}
