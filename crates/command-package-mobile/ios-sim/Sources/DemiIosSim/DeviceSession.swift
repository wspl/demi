// One attached simulator: its framebuffer, its HID connection, the
// orientation demi-mobile set, its captures and its recording.
//
// The simulator turns its screen but not its framebuffer, which stays
// portrait with the picture sideways, so every picture is turned upright
// here and every point the caller gives, in the upright screen's points, is
// turned back into the portrait points the HID takes
// (`crates/command-package-mobile/src/ios/mod.rs`).

import CoreImage
import FBControlCore
import FBSimulatorAX
import FBSimulatorControl
import FBSimulatorServices
import Foundation
import IOSurface
import QuartzCore

/// Which way the device is turned, as demi-mobile names it.
enum Turn: String {
  case portrait
  case landscapeLeft
  case portraitUpsideDown
  case landscapeRight

  var device: SimulatorDeviceOrientation {
    switch self {
    case .portrait: return .portrait
    case .landscapeLeft: return .landscapeLeft
    case .portraitUpsideDown: return .portraitUpsideDown
    case .landscapeRight: return .landscapeRight
    }
  }

  var landscape: Bool { self == .landscapeLeft || self == .landscapeRight }

  /// The quarter turns clockwise the device's frame is drawn at.
  var frameTurns: Int {
    switch self {
    case .portrait: return 0
    case .landscapeLeft: return 3
    case .portraitUpsideDown: return 2
    case .landscapeRight: return 1
    }
  }

  /// How the framebuffer's portrait picture is shown upright.
  var picture: CGImagePropertyOrientation {
    switch self {
    case .portrait: return .up
    case .landscapeLeft: return .left
    case .portraitUpsideDown: return .down
    case .landscapeRight: return .right
    }
  }
}

final class DeviceSession: @unchecked Sendable {
  let simulator: Simulator
  let udid: String
  private let output: Output
  private let hid: SimulatorHIDSession
  private let attachment: FramebufferAttachment
  /// The screen in portrait points.
  private let portraitPoints: CGSize
  private let lock = NSLock()
  private var surface: IOSurface?
  private var turn = Turn.portrait
  private var captures: [UInt32: Encoder] = [:]
  private var recorder: Recorder?
  private var detached = false
  /// The UI automation mode before the guest bridge first turned it on,
  /// which detaching restores: nil while the bridge was never used.
  private var automationBefore: AutomationBefore?
  private var lastDrawn: CFTimeInterval = 0
  private var drawnPending = false
  let pictures = Pictures()

  private init(
    simulator: Simulator, output: Output, hid: SimulatorHIDSession, attachment: FramebufferAttachment,
    portraitPoints: CGSize
  ) {
    self.simulator = simulator
    self.udid = simulator.udid
    self.output = output
    self.hid = hid
    self.attachment = attachment
    self.portraitPoints = portraitPoints
    self.surface = attachment.initialSurface
  }

  /// Attaches the booted simulator `udid` and turns it portrait.
  static func attach(udid: String, output: Output) async throws -> DeviceSession {
    let simulator = try Simulators.named(udid)
    guard simulator.state == .booted else {
      throw HelperError("\(simulator.name) is not booted")
    }
    let framebuffer = try await simulator.framebuffer.connect()
    let attachment = try framebuffer.attach()
    guard let surface = attachment.initialSurface else {
      attachment.cancel()
      throw HelperError("\(simulator.name) has no screen to show")
    }
    let scale = Double(simulator.screenInfo?.scale ?? 3)
    let points = CGSize(
      width: Double(IOSurfaceGetWidth(surface)) / scale, height: Double(IOSurfaceGetHeight(surface)) / scale)
    let hid: SimulatorHIDSession
    do {
      hid = try await SimulatorHIDSession.open(simulator)
    } catch {
      attachment.cancel()
      throw error
    }
    let session = DeviceSession(
      simulator: simulator, output: output, hid: hid, attachment: attachment, portraitPoints: points)
    do {
      // The simulator cannot say which way it is turned, so it starts from
      // the orientation set here.
      try await simulator.orientation.set(.portrait)
    } catch {
      await session.detach()
      throw error
    }
    session.follow()
    return session
  }

  /// The screen, upright: its points and its pixels.
  func screenJSON() -> [String: Any] {
    let (points, pixels) = lock.withLock { () -> (CGSize, CGSize) in
      let pixels = surface.map { CGSize(width: IOSurfaceGetWidth($0), height: IOSurfaceGetHeight($0)) } ?? .zero
      return turn.landscape
        ? (CGSize(width: portraitPoints.height, height: portraitPoints.width),
          CGSize(width: pixels.height, height: pixels.width))
        : (portraitPoints, pixels)
    }
    return [
      "points": ["width": points.width, "height": points.height],
      "pixels": ["width": Int(pixels.width), "height": Int(pixels.height)],
    ]
  }

  // MARK: Pictures

  /// Follows the framebuffer: each new picture is drawn, encoded for each
  /// capture and added to the recording; its end is the device's loss.
  private func follow() {
    Task { [self] in
      for await event in attachment.events {
        switch event {
        case let .surfaceChanged(changed):
          lock.withLock { surface = changed }
          drawn()
        case .frameRendered:
          drawn()
        case let .ended(error):
          lost(describe(error))
          return
        case .configurationChanged:
          break
        @unknown default:
          break
        }
      }
      lost("its screen stopped")
    }
  }

  private func lost(_ reason: String) {
    let report = lock.withLock { () -> Bool in
      let report = !detached
      detached = true
      return report
    }
    if report {
      output.control(["event": "lost", "udid": udid, "reason": reason])
    }
  }

  /// The upright picture now; nil while the device shows none.
  func upright() -> CIImage? {
    let (surface, turn) = lock.withLock { (self.surface, self.turn) }
    guard let surface else { return nil }
    return CIImage(ioSurface: surface).oriented(turn.picture)
  }

  private func drawn() {
    let (encoders, recorder) = lock.withLock { (Array(captures.values), self.recorder) }
    for encoder in encoders { encoder.drawn() }
    recorder?.drawn()
    tellDrawn()
  }

  /// Tells demi-mobile a new picture was drawn, at most every 50 ms, the
  /// last change always told.
  private func tellDrawn() {
    let interval = 0.05
    let wait = lock.withLock { () -> Double? in
      let now = CACurrentMediaTime()
      if now - lastDrawn >= interval {
        lastDrawn = now
        return 0
      }
      if drawnPending { return nil }
      drawnPending = true
      return interval - (now - lastDrawn)
    }
    guard let wait else { return }
    if wait == 0 {
      output.control(["event": "drawn", "udid": udid])
      return
    }
    DispatchQueue.global().asyncAfter(deadline: .now() + wait) { [self] in
      lock.withLock {
        drawnPending = false
        lastDrawn = CACurrentMediaTime()
      }
      output.control(["event": "drawn", "udid": udid])
    }
  }

  func screenshot(longSide: Int) throws -> (Data, Int, Int) {
    guard let image = upright() else { throw HelperError("\(simulator.name) shows no picture") }
    return try pictures.png(image, longSide: longSide)
  }

  // MARK: The frame

  /// The device's frame for a view of `area` pixels at `backing` pixels
  /// per point, turned as the picture is, with the device's silhouette that
  /// bounds the picture under it; the screen's corner alone, in the screen's
  /// points, for the frame the view draws, when Xcode installed no frame for
  /// the device type.
  func frame(area: CGSize, backing: Double) throws -> ([String: Any], Data?) {
    let (turn, surface) = lock.withLock { (self.turn, self.surface) }
    let looks = DeviceLooks(simulator: simulator)
    if let drawn = try drawFrame(looks: looks, screenPoints: portraitPoints, quarterTurns: turn.frameTurns, area: area, backing: backing) {
      return (
        [
          "width": drawn.width, "height": drawn.height,
          "screen": [
            "x": drawn.screen.minX, "y": drawn.screen.minY, "width": drawn.screen.width,
            "height": drawn.screen.height,
          ],
          // The blob is the frame's PNG, then the silhouette's.
          "pngBytes": drawn.png.count,
        ], drawn.png + drawn.silhouette
      )
    }
    let pixels = surface.map { CGSize(width: IOSurfaceGetWidth($0), height: IOSurfaceGetHeight($0)) } ?? .zero
    let corner = looks.corner(pixels: pixels, screenPoints: portraitPoints, quarterTurns: turn.frameTurns)
    let scale = pixels.width > 0 ? portraitPoints.width / pixels.width : 0
    return (["corner": cornerPoints(corner, scale: scale)], nil)
  }

  /// `corner`'s points at `scale`, as the reply carries them.
  private func cornerPoints(_ corner: [CGPoint], scale: Double) -> [[String: Double]] {
    corner.map { ["x": $0.x * scale, "y": $0.y * scale] }
  }

  // MARK: Input

  /// The portrait point the HID takes for the upright point `x`, `y`.
  private func devicePoint(_ x: Double, _ y: Double) -> (Double, Double) {
    let turn = lock.withLock { self.turn }
    let width = portraitPoints.width
    let height = portraitPoints.height
    switch turn {
    case .portrait: return (x, y)
    case .landscapeLeft: return (width - y, x)
    case .portraitUpsideDown: return (width - x, height - y)
    case .landscapeRight: return (y, height - x)
    }
  }

  private func send(_ event: SimulatorHIDEvent) async throws {
    try await hid.hid.send(event: event, logger: simulator.logger)
  }

  func touch(phase: String, x: Double, y: Double) async throws {
    let (px, py) = devicePoint(x, y)
    switch phase {
    // A finger that moves is a contact still down, at its new place.
    case "down", "move": try await send(.touch(direction: .down, x: px, y: py))
    case "up": try await send(.touch(direction: .up, x: px, y: py))
    default: throw HelperError("a touch is down, move or up, not \(phase)")
    }
  }

  func tap(x: Double, y: Double, seconds: Double) async throws {
    let (px, py) = devicePoint(x, y)
    try await send(seconds > 0 ? .tapAt(x: px, y: py, duration: seconds) : .tapAt(x: px, y: py))
  }

  func swipe(from: CGPoint, to: CGPoint) async throws {
    let (x1, y1) = devicePoint(from.x, from.y)
    let (x2, y2) = devicePoint(to.x, to.y)
    try await send(
      .swipe(x1, yStart: y1, xEnd: x2, yEnd: y2, delta: SimulatorHIDEvent.defaultSwipeDelta, duration: 0.3))
  }

  func button(_ name: String) async throws {
    let button: SimulatorHIDButton
    switch name {
    case "home": button = .homeButton
    case "lock": button = .lock
    case "volumeUp": button = .volumeUp
    case "volumeDown": button = .volumeDown
    default: throw HelperError("an iPhone has no \(name) button")
    }
    try await send(.shortButtonPress(button))
  }

  func key(usage: UInt32, down: Bool) async throws {
    try await send(.keyboard(direction: down ? .down : .up, keyCode: usage))
  }

  /// Types each key, holding Shift for those that need it.
  func type(_ keys: [(UInt32, Bool)]) async throws {
    let shift: UInt32 = 225
    var events: [SimulatorHIDEvent] = []
    for (usage, shifted) in keys {
      if shifted { events.append(.keyboard(direction: .down, keyCode: shift)) }
      events.append(.keyboard(direction: .down, keyCode: usage))
      events.append(.keyboard(direction: .up, keyCode: usage))
      if shifted { events.append(.keyboard(direction: .up, keyCode: shift)) }
    }
    try await send(.composite(events))
  }

  /// Sets the text of the field at the upright point `x`, `y` at once.
  func setValue(_ text: String, x: Double, y: Double) async throws {
    let automation = try simulator.uiAutomation(backend: .accessibility)
    try await automation.setValue(text, for: .point(CGPoint(x: x, y: y)))
  }

  // MARK: The device

  func orient(_ name: String) async throws {
    guard let turn = Turn(rawValue: name) else {
      throw HelperError("\(name) is no orientation")
    }
    try await simulator.orientation.set(turn.device)
    lock.withLock { self.turn = turn }
    // The view and the captures follow the new size; the picture they
    // draw next is turned the new way.
    drawn()
  }

  func appearance() async throws -> String {
    try await simulator.preferences.currentSettingValue(name: "appearance", domain: nil)
  }

  func setAppearance(_ name: String) async throws {
    switch name {
    case "light": try await simulator.preferences.apply(.appearance(.light))
    case "dark": try await simulator.preferences.apply(.appearance(.dark))
    default: throw HelperError("the appearance is light or dark, not \(name)")
    }
  }

  // MARK: The tree

  func tree() async throws -> [[String: Any]] {
    try await Tree(session: self).read()
  }

  /// Records the UI automation mode before the guest bridge's first use,
  /// which turns it on.
  func beforeBridge() async {
    let recorded = lock.withLock { automationBefore != nil }
    if recorded { return }
    let mode = await AutomationMode(simulator: simulator).read()
    lock.withLock {
      if automationBefore == nil { automationBefore = mode }
    }
  }

  // MARK: Captures and the recording

  func captureStart(capture: UInt32, width: Int, height: Int, fps: Int, bitrate: Int) throws {
    let encoder = try Encoder(
      capture: capture, width: width, height: height, fps: fps, bitrate: bitrate, session: self, output: output)
    lock.withLock { captures[capture] = encoder }
    encoder.keyFrame()
  }

  func captureKeyFrame(capture: UInt32) {
    lock.withLock { captures[capture] }?.keyFrame()
  }

  func captureAck(capture: UInt32, sequence: UInt32, window: UInt32, order: UInt64) {
    lock.withLock { captures[capture] }?.ack(sequence: sequence, window: window, order: order)
  }

  func captureEncoding(capture: UInt32, bitrate: Int, fps: Int) {
    lock.withLock { captures[capture] }?.encoding(bitrate: bitrate, fps: fps)
  }

  func captureStop(capture: UInt32) {
    lock.withLock { captures.removeValue(forKey: capture) }?.stop()
  }

  /// Records into `path`, which demi-mobile chose in the conversation's
  /// scratch directory and takes once the recording stops.
  func recordStart(path: String) throws {
    guard let image = upright() else { throw HelperError("\(simulator.name) shows no picture") }
    let recorder = try Recorder(size: image.extent.size, url: URL(fileURLWithPath: path), session: self)
    lock.withLock { self.recorder = recorder }
    recorder.drawn()
  }

  /// Ends the recording; answers its length in milliseconds.
  func recordStop() async throws -> Int {
    guard let recorder = lock.withLock({ () -> Recorder? in
      defer { self.recorder = nil }
      return self.recorder
    }) else {
      throw HelperError("\(simulator.name) is not recording")
    }
    return try await recorder.finish()
  }

  /// Lets the device go: its captures and recording stop, the automation
  /// mode the bridge changed is restored, the HID and the framebuffer
  /// disconnect.
  func detach() async {
    let (encoders, recorder, automation) = lock.withLock {
      () -> ([Encoder], Recorder?, AutomationBefore?) in
      detached = true
      defer {
        captures.removeAll()
        self.recorder = nil
      }
      return (Array(captures.values), self.recorder, automationBefore)
    }
    for encoder in encoders { encoder.stop() }
    if let recorder { await recorder.discard() }
    if let automation {
      do {
        try await AutomationMode(simulator: simulator).restore(automation)
      } catch {
        log("\(simulator.name): the UI automation mode could not be restored: \(describe(error))")
      }
    }
    await hid.close()
    attachment.cancel()
  }
}
