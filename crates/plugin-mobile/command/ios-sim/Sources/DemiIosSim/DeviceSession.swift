// One attached simulator: its framebuffer, its HID connection, the
// orientation its screen shows, its captures and its recording.
//
// The simulator turns its screen but not its framebuffer, which stays
// portrait with the picture sideways, so every picture is turned upright
// here and every point the caller gives, in the upright screen's points, is
// turned back into the portrait points the HID takes
// (`crates/plugin-mobile/command/src/ios/mod.rs`).
//
// The screen's orientation is the one its display shows, which the app in
// front decides: turning the device asks for a turn, and an app that does
// not allow it, as most iPhone apps refuse upside down, keeps the screen as
// it was. The session follows the display and tells demi-mobile of each turn
// it takes, whoever asked for it.

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

  /// The orientation a display turned by `rotation` shows; none for a
  /// rotation a later Xcode adds, which no phone's screen takes.
  init?(_ rotation: SimulatorDisplayRotation) {
    switch rotation {
    case .upright: self = .portrait
    case .clockwise: self = .landscapeRight
    case .upsideDown: self = .portraitUpsideDown
    case .counterclockwise: self = .landscapeLeft
    @unknown default: return nil
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
  /// The orientation the screen shows, as its display last said.
  private var turn = Turn.portrait
  /// Those waiting for the screen to show an orientation, by their identity.
  private var turnWaiters: [UUID: (Turn, CheckedContinuation<Void, Never>)] = [:]
  private var followingTurns: Task<Void, Never>?
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
    session.follow()
    do {
      // The device is held upright, as a phone picked up is.
      try await session.orient(.portrait)
    } catch {
      await session.detach()
      throw error
    }
    return session
  }

  /// The screen, upright: its points and its pixels.
  func screenJSON() -> [String: Any] {
    let (turn, surface) = lock.withLock { (self.turn, self.surface) }
    return screenJSON(turn: turn, surface: surface)
  }

  /// The screen upright when turned `turn`: its orientation, its points
  /// and its pixels.
  private func screenJSON(turn: Turn, surface: IOSurface?) -> [String: Any] {
    let pixels = surface.map { CGSize(width: IOSurfaceGetWidth($0), height: IOSurfaceGetHeight($0)) } ?? .zero
    let (points, upright) =
      turn.landscape
      ? (CGSize(width: portraitPoints.height, height: portraitPoints.width),
        CGSize(width: pixels.height, height: pixels.width))
      : (portraitPoints, pixels)
    return [
      "orientation": turn.rawValue,
      "points": ["width": points.width, "height": points.height],
      "pixels": ["width": Int(upright.width), "height": Int(upright.height)],
    ]
  }

  // MARK: Pictures

  /// Follows the framebuffer: each new picture is drawn, encoded for each
  /// capture and added to the recording; its end is the device's loss. Follows
  /// the display: each turn it takes is the screen's.
  private func follow() {
    let display = SimulatorDisplayCommands(simulator: simulator)
    let following = Task { [self] in
      for await configuration in display.configurations() {
        guard case .settled = configuration.phase, let geometry = try? configuration.activeGeometry(),
          let turn = Turn(geometry.rotation)
        else {
          continue
        }
        turned(turn)
      }
    }
    lock.withLock { followingTurns = following }
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

  /// The display shows `turn`: the pictures, the points the caller gives
  /// and the frame turn with it, demi-mobile hears of it, and those waiting
  /// for it go on.
  private func turned(_ turn: Turn) {
    let (changed, surface, waiting) = lock.withLock { () -> (Bool, IOSurface?, [CheckedContinuation<Void, Never>]) in
      let changed = self.turn != turn
      self.turn = turn
      let waiting = turnWaiters.filter { $0.value.0 == turn }
      for id in waiting.keys { turnWaiters[id] = nil }
      return (changed, self.surface, waiting.values.map { $0.1 })
    }
    if changed {
      output.control(["event": "turned", "udid": udid, "screen": screenJSON(turn: turn, surface: surface)])
      // The captures follow the new size; the picture they draw next is
      // turned the new way.
      drawn()
    }
    for waiter in waiting { waiter.resume() }
  }

  /// Returns once the screen shows `target`, or after `seconds`: the app in
  /// front may refuse the turn.
  private func screenShows(_ target: Turn, within seconds: Double) async {
    await withTaskGroup(of: Void.self) { group in
      group.addTask { [self] in await shows(target) }
      group.addTask { try? await Task.sleep(for: .seconds(seconds)) }
      await group.next()
      group.cancelAll()
    }
  }

  /// Returns once the screen shows `target`, or once the task is cancelled.
  private func shows(_ target: Turn) async {
    let id = UUID()
    await withTaskCancellationHandler {
      await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
        let now = lock.withLock { () -> Bool in
          if turn == target || Task.isCancelled { return true }
          turnWaiters[id] = (target, continuation)
          return false
        }
        if now { continuation.resume() }
      }
    } onCancel: { [self] in
      lock.withLock { turnWaiters.removeValue(forKey: id) }?.1.resume()
    }
  }

  // MARK: The frame

  /// The device's frame for a view of `area` pixels at `backing` pixels
  /// per point, turned as the picture is, with the device's silhouette that
  /// bounds the picture under it; the screen's corner alone, in the screen's
  /// points, for the frame the view draws, when Xcode installed no frame for
  /// the device type.
  ///
  /// The reply names the screen the frame was drawn for, its orientation and
  /// size, read with what it is drawn from, so a turn meanwhile never pairs
  /// a frame with another orientation.
  func frame(area: CGSize, backing: Double) throws -> ([String: Any], Data?) {
    let (turn, surface) = lock.withLock { (self.turn, self.surface) }
    let screen = screenJSON(turn: turn, surface: surface)
    let looks = DeviceLooks(simulator: simulator)
    if let drawn = try drawFrame(looks: looks, screenPoints: portraitPoints, quarterTurns: turn.frameTurns, area: area, backing: backing) {
      return (
        [
          "for": screen,
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
    return (["for": screen, "corner": cornerPoints(corner, scale: scale)], nil)
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

  /// Turns the device to `name` and returns once the screen took the turn,
  /// or once it is clear that the app in front refused it.
  func orient(_ name: String) async throws {
    guard let turn = Turn(rawValue: name) else {
      throw HelperError("\(name) is no orientation")
    }
    try await orient(turn)
  }

  private func orient(_ turn: Turn) async throws {
    try await simulator.orientation.set(turn.device)
    // The display turns about half a second after the device; one that has
    // not turned within twice that keeps its orientation.
    await screenShows(turn, within: 1.5)
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
    lock.withLock { followingTurns }?.cancel()
    await hid.close()
    attachment.cancel()
  }
}
