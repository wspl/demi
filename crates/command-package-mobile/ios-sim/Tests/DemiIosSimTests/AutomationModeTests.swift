// Detaching puts the UI automation mode the guest bridge turned on back as
// it was, against a guest `defaults` that behaves as iOS 26.5's does
// (`mobile.md` § iOS).

import FBSimulatorControl
import Foundation
import XCTest

@testable import DemiIosSim

final class AutomationModeTests: XCTestCase {
  /// The guest's `defaults` for the one key: `read` prints a Boolean as 1 or
  /// 0, or fails with exit 1 for a missing pair; `write -bool` takes only
  /// true, false, yes or no and fails with 255 for 0 or 1; `delete` removes
  /// the key. `unreadable` makes `read` fail as a broken guest would.
  final class Guest {
    var value: Bool?
    var unreadable = false
    init(_ value: Bool?) { self.value = value }

    func run(_ arguments: [String]) -> InSimulatorToolOutput {
      func exit(_ code: Int32, _ stdout: String = "", _ stderr: String = "") -> InSimulatorToolOutput {
        InSimulatorToolOutput(stdout: Data(stdout.utf8), stderr: Data(stderr.utf8), exitCode: code)
      }
      switch arguments {
      case ["read", AutomationMode.domain, AutomationMode.key]:
        if unreadable { return exit(70, "", "Could not connect to cfprefsd") }
        guard let value else {
          return exit(1, "", "The domain/default pair of (com.apple.Accessibility, AutomationEnabled) does not exist")
        }
        return exit(0, value ? "1\n" : "0\n")
      case ["write", AutomationMode.domain, AutomationMode.key, "-bool", arguments.last!]:
        switch arguments.last! {
        case "true", "yes": value = true
        case "false", "no": value = false
        default: return exit(255, "", "Command line interface to a user's defaults.")
        }
        return exit(0)
      case ["delete", AutomationMode.domain, AutomationMode.key]:
        if value == nil { return exit(1, "", "Domain (com.apple.Accessibility) not found.") }
        value = nil
        return exit(0)
      default:
        return exit(255, "", "unexpected defaults \(arguments)")
      }
    }
  }

  /// Reads the mode as the bridge's first use does, turns it on as the
  /// bridge does, and restores it as detaching does; answers the restore's
  /// failure, if any.
  func bridgeAndDetach(_ guest: Guest) async -> Error? {
    let mode = AutomationMode { guest.run($0) }
    let before = await mode.read()
    guest.value = true
    do {
      try await mode.restore(before)
      return nil
    } catch {
      return error
    }
  }

  /// Off before is off again, written in the syntax the guest takes, where
  /// the numeric 0 a read prints fails. Planted defect this catches: the
  /// value written as read. Milliseconds.
  func testAModeThatWasOffIsOffAgain() async {
    let guest = Guest(false)
    let failure = await bridgeAndDetach(guest)
    XCTAssertNil(failure)
    XCTAssertEqual(guest.value, false)
  }

  /// On before stays on, written as the guest takes it. Planted defect
  /// this catches: the 1 a read prints written back. Milliseconds.
  func testAModeThatWasOnIsOnAgain() async {
    let guest = Guest(true)
    let failure = await bridgeAndDetach(guest)
    XCTAssertNil(failure)
    XCTAssertEqual(guest.value, true)
  }

  /// No value before is no value again, not off. Planted defects this
  /// catches: an absent value restored as off; an absent value taken for a
  /// failed read. Milliseconds.
  func testAModeTheGuestDidNotHaveIsDeletedAgain() async {
    let guest = Guest(nil)
    let failure = await bridgeAndDetach(guest)
    XCTAssertNil(failure)
    XCTAssertNil(guest.value)
  }

  /// A read that failed restores nothing and says so, rather than guessing
  /// off. Planted defect this catches: a failed read taken for an absent or
  /// an off value. Milliseconds.
  func testAModeThatCouldNotBeReadIsLeftAndSaysSo() async throws {
    let guest = Guest(false)
    guest.unreadable = true
    let restored = await bridgeAndDetach(guest)
    let failure = try XCTUnwrap(restored)
    XCTAssertEqual(guest.value, true)
    XCTAssertTrue(describe(failure).contains("could not be read"), describe(failure))
  }
}
