// The iOS helper of `demi mobile` (`mobile.md` § iOS). One-shot commands
// list, boot and shut down the Mac's simulators and print JSON; `serve`
// drives the simulators a conversation opened, speaking demi-mobile's
// framing over standard input and output.

import FBControlCore
import FBSimulatorControl
import Foundation

@main
struct DemiIosSim {
  static func main() async {
    let arguments = Array(CommandLine.arguments.dropFirst())
    do {
      switch (arguments.first, arguments.count) {
      case ("list", 1):
        try printJSON(try Simulators.list())
      case ("boot", 2):
        try await Simulators.boot(arguments[1])
        try printJSON([String: String]())
      case ("shutdown", 2):
        try await Simulators.shutdown(arguments[1])
        try printJSON([String: String]())
      case ("serve", 1):
        try await Server().run()
      default:
        fail("usage: demi-ios-sim list | boot <udid> | shutdown <udid> | serve")
      }
    } catch {
      fail(describe(error))
    }
    exit(0)
  }
}

/// Writes `value` as JSON on standard output.
func printJSON<T: Encodable>(_ value: T) throws {
  FileHandle.standardOutput.write(try JSONEncoder().encode(value))
}

/// Says `message` on standard error, whose last line demi-mobile reads, and
/// exits with failure.
func fail(_ message: String) -> Never {
  log(message)
  exit(1)
}

/// A line on standard error, which demi-mobile keeps in the Host's log.
func log(_ message: String) {
  FileHandle.standardError.write(Data((message + "\n").utf8))
}

/// What a failure says, in one line.
func describe(_ error: Error) -> String {
  let text = (error as? LocalizedError)?.errorDescription ?? "\(error)"
  return text.replacingOccurrences(of: "\n", with: " ")
}

/// A failure the helper words itself.
struct HelperError: Error, LocalizedError {
  let message: String
  init(_ message: String) { self.message = message }
  var errorDescription: String? { message }
}
