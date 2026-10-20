// The simulator's UI automation mode, which the guest bridge turns on: what
// it was before the bridge's first use, and putting it back when the device
// is let go (`crates/plugin-mobile/command/src/ios/mod.rs`).
//
// idb's preference read answers an empty value both for a key the guest
// does not have and for a read that failed, and idb cannot delete a key, so
// the helper runs the guest's own `defaults` for this setting.

import FBSimulatorControl
import Foundation

/// The automation mode as the bridge found it.
enum AutomationBefore: Equatable {
  case on
  case off
  /// The guest had no value of its own.
  case absent
  /// The read failed, with why; nothing is restored then.
  case unread(String)
}

struct AutomationMode {
  static let key = "AutomationEnabled"
  static let domain = "com.apple.Accessibility"

  /// Runs the guest's `defaults` with these arguments.
  let defaults: ([String]) async throws -> InSimulatorToolOutput

  init(defaults: @escaping ([String]) async throws -> InSimulatorToolOutput) {
    self.defaults = defaults
  }

  init(simulator: Simulator) {
    self.init { try await simulator.runtimeTools.run("usr/bin/defaults", arguments: $0) }
  }

  func read() async -> AutomationBefore {
    let arguments = ["read", Self.domain, Self.key]
    let output: InSimulatorToolOutput
    do {
      output = try await defaults(arguments)
    } catch {
      return .unread(describe(error))
    }
    if output.exitCode == 0 {
      // `defaults read` prints a Boolean as 1 or 0.
      switch Self.text(output.stdout) {
      case "1": return .on
      case "0": return .off
      case let value: return .unread("defaults read printed \(value), which is no Boolean")
      }
    }
    // `defaults` exits with 1 for a pair the guest does not have and for
    // other failures alike; only its message tells them apart.
    if output.exitCode == 1, Self.text(output.stderr).contains("does not exist") { return .absent }
    return .unread(Self.failure(arguments, output))
  }

  /// Puts the mode back as it was `before`: on or off as the guest's
  /// `defaults` spells a Boolean it writes, true or false, and an absent
  /// value by deleting the one the bridge wrote.
  func restore(_ before: AutomationBefore) async throws {
    let arguments: [String]
    switch before {
    case .on: arguments = ["write", Self.domain, Self.key, "-bool", "true"]
    case .off: arguments = ["write", Self.domain, Self.key, "-bool", "false"]
    case .absent: arguments = ["delete", Self.domain, Self.key]
    case let .unread(why):
      throw HelperError("what it was before could not be read (\(why)), so it is left as it is")
    }
    let output = try await defaults(arguments)
    if output.exitCode != 0 { throw HelperError(Self.failure(arguments, output)) }
  }

  private static func text(_ data: Data) -> String {
    String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
  }

  private static func failure(_ arguments: [String], _ output: InSimulatorToolOutput) -> String {
    let stderr = text(output.stderr).replacingOccurrences(of: "\n", with: " ")
    return "defaults \(arguments.joined(separator: " ")) exited with \(output.exitCode): \(stderr)"
  }
}
