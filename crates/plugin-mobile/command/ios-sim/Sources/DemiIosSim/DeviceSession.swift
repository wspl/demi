// One attached simulator: its framebuffer, its HID connection, the
// orientation its screen shows, its captures and its recordings.
//
// A foldable, as iPhone Duo, has two displays, of which the guest lights
// one: the outer display, the device's primary one, while closed, the inner
// one open. The session attaches the framebuffer to the lit display and
// again whenever the guest lights the other, whoever moved the hinge; the
// HID and the tree go to the lit display by themselves. A display's own
// turn while the device is upright, a quarter turn for iPhone Duo's inner
// display, is taken off the orientation the screen reports, so an open
// device held upright is in portrait.
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

  /// Quarter turns clockwise from portrait.
  var quarters: Int {
    switch self {
    case .portrait: return 0
    case .landscapeRight: return 1
    case .portraitUpsideDown: return 2
    case .landscapeLeft: return 3
    }
  }

  init(quarters: Int) {
    switch (quarters % 4 + 4) % 4 {
    case 1: self = .landscapeRight
    case 2: self = .portraitUpsideDown
    case 3: self = .landscapeLeft
    default: self = .portrait
    }
  }

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

/// The device's own displays, as a settled configuration names them: each
/// one's turn while the device is upright and, for a foldable, which is
/// outside and which inside.
struct DisplayLayout {
  /// The quarter turns clockwise each display shows while the device is
  /// upright, by its unique ID: the turn beyond the primary display's, which
  /// turns with the device.
  let natural: [String: Int]
  /// A foldable's outer display, its primary one, and its inner display.
  let fold: (outer: SimulatorDisplay, inner: SimulatorDisplay)?

  init(_ configuration: SimulatorDisplayConfiguration) {
    let integrated = configuration.displays.filter(\.isIntegrated)
    let primary = integrated.first(where: \.isPrimary)
    let base = primary.flatMap { Turn($0.rotation)?.quarters } ?? 0
    natural = Dictionary(
      integrated.map { ($0.uniqueID, ((Turn($0.rotation)?.quarters ?? 0) - base + 4) % 4) },
      uniquingKeysWith: { first, _ in first })
    if integrated.count == 2, let primary, let inner = integrated.first(where: { !$0.isPrimary }) {
      fold = (primary, inner)
    } else {
      fold = nil
    }
  }

  /// `display`'s size in points while the device is upright.
  func uprightPoints(_ display: SimulatorDisplay) -> CGSize {
    let size = CGSize(width: display.bounds.width / display.scale, height: display.bounds.height / display.scale)
    return (natural[display.uniqueID] ?? 0) % 2 == 1 ? CGSize(width: size.height, height: size.width) : size
  }
}

/// Which way a foldable's hinge is, as demi-mobile names its displays.
enum FoldDisplay: String {
  case outer
  case inner
}

/// The screen upright, in points, and the quarter turns clockwise its
/// display shows while the device is upright.
struct ScreenTurn {
  let upright: CGSize
  let natural: Int
}

/// The space an accessibility read's frames and points are in: upright, or
/// the upright screen turned back by the lit display's own turn, as
/// SpringBoard reads on iPhone Duo's inner display, in its portrait points.
struct AccessibilitySpace {
  /// The quarter turns clockwise from the read's space to the upright
  /// screen.
  let quarters: Int
  /// The read's space, in points.
  let size: CGSize

  /// `rect` of the read, upright.
  func upright(_ rect: CGRect) -> CGRect {
    var rect = rect
    var size = size
    for _ in 0..<quarters {
      // A quarter turn clockwise: the left edge becomes the top.
      rect = CGRect(x: size.height - rect.maxY, y: rect.minX, width: rect.height, height: rect.width)
      size = CGSize(width: size.height, height: size.width)
    }
    return rect
  }

  /// The upright point `point` in the read's space.
  func read(_ point: CGPoint) -> CGPoint {
    // Turned back a quarter at a time: clockwise once is counterclockwise
    // three times.
    var point = point
    var upright = quarters % 2 == 1 ? CGSize(width: size.height, height: size.width) : size
    for _ in 0..<((4 - quarters) % 4) {
      point = CGPoint(x: upright.height - point.y, y: point.x)
      upright = CGSize(width: upright.height, height: upright.width)
    }
    return point
  }
}

final class DeviceSession: @unchecked Sendable {
  let simulator: Simulator
  let udid: String
  private let output: Output
  private let hid: SimulatorHIDSession
  private let layout: DisplayLayout?
  private let lock = NSLock()
  /// The lit display's framebuffer.
  private var attachment: FramebufferAttachment
  /// The lit display, by its unique ID; none for a device whose display
  /// idb does not name.
  private var display: String?
  /// The screen in the points of its framebuffer, which stays upright
  /// whichever way the screen turns.
  private var portraitPoints: CGSize
  private var surface: IOSurface?
  /// The turn the lit display's picture shows, as its display last said.
  private var turn = Turn.portrait
  /// The quarter turns clockwise the lit display shows while the device is
  /// upright, which the orientation the screen reports leaves out.
  private var natural = 0
  /// A foldable's hinge, opened at its first fold.
  private var hinge: Hinge?
  /// The hinge's angle as Demi last moved it, or as the lit display says
  /// after another folded the device: flat with the inner display lit,
  /// closed with the outer one. The guest reports no angle.
  private var degrees = 0
  /// Whether the session moves the hinge, while the angle is its own.
  private var folding = false
  /// Those waiting for the screen to show an orientation, by their identity.
  private var turnWaiters: [UUID: (Turn, CheckedContinuation<Void, Never>)] = [:]
  /// The space the last tree read in; none before the first.
  private var readSpace: AccessibilitySpace?
  /// Those waiting for a display to be lit, by their identity.
  private var displayWaiters: [UUID: (String, CheckedContinuation<Void, Never>)] = [:]
  private var followingTurns: Task<Void, Never>?
  private var captures: [UInt32: Encoder] = [:]
  private var taps: [UInt32: RecordingTap] = [:]
  private var detached = false
  /// The UI automation mode before the guest bridge first turned it on,
  /// which detaching restores: nil while the bridge was never used.
  private var automationBefore: AutomationBefore?
  private var lastDrawn: CFTimeInterval = 0
  private var drawnPending = false
  let pictures = Pictures()

  private init(
    simulator: Simulator, output: Output, hid: SimulatorHIDSession, layout: DisplayLayout?,
    lit: SimulatorDisplay?, attachment: FramebufferAttachment, portraitPoints: CGSize
  ) {
    self.simulator = simulator
    self.udid = simulator.udid
    self.output = output
    self.hid = hid
    self.layout = layout
    self.attachment = attachment
    self.portraitPoints = portraitPoints
    self.surface = attachment.initialSurface
    self.display = lit?.uniqueID
    self.natural = lit.flatMap { layout?.natural[$0.uniqueID] } ?? 0
    if let lit, let fold = layout?.fold {
      self.degrees = lit.uniqueID == fold.inner.uniqueID ? FLAT_DEGREES : 0
    }
  }

  /// Attaches the booted simulator `udid` and turns it portrait.
  static func attach(udid: String, output: Output) async throws -> DeviceSession {
    let simulator = try Simulators.named(udid)
    guard simulator.state == .booted else {
      throw HelperError("\(simulator.name) is not booted")
    }
    // A display change still under way names no active display: the lit
    // one is the integrated display that is on, which `follow` moves from
    // once the change settles.
    let configuration = try await SimulatorDisplayCommands(simulator: simulator).configuration(
      settledWithin: .seconds(2))
    let lit =
      (try? configuration.activeDisplay())
      ?? configuration.displays.first { $0.isIntegrated && $0.isActive }
    let (attachment, surface) = try await framebuffer(simulator, on: lit.map { .display(uniqueID: $0.uniqueID) } ?? .main)
    let scale = lit?.scale ?? Double(simulator.screenInfo?.scale ?? 3)
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
      simulator: simulator, output: output, hid: hid, layout: lit.map { _ in DisplayLayout(configuration) }, lit: lit,
      attachment: attachment, portraitPoints: points)
    session.follow()
    await session.skipKeyboardTutorials()
    do {
      // The device is held upright, as a phone picked up is.
      try await session.orient(.portrait)
    } catch {
      await session.detach()
      throw error
    }
    return session
  }

  /// The lit display's framebuffer, attached.
  private static func framebuffer(_ simulator: Simulator, on display: DisplaySelection) async throws
    -> (FramebufferAttachment, IOSurface)
  {
    let framebuffer = try await simulator.framebuffer.connect(display: display)
    let attachment = try framebuffer.attach()
    guard let surface = attachment.initialSurface else {
      attachment.cancel()
      throw HelperError("\(simulator.name) has no screen to show")
    }
    return (attachment, surface)
  }

  /// The screen, upright: its orientation, its points, its pixels and, for
  /// a foldable, how it is folded.
  func screenJSON() -> [String: Any] {
    lock.withLock { screenJSONLocked() }
  }

  /// The screen as `screenJSON` says it, read under the lock.
  private func screenJSONLocked() -> [String: Any] {
    let pixels = surface.map { CGSize(width: IOSurfaceGetWidth($0), height: IOSurfaceGetHeight($0)) } ?? .zero
    let (points, upright) =
      turn.landscape
      ? (CGSize(width: portraitPoints.height, height: portraitPoints.width),
        CGSize(width: pixels.height, height: pixels.width))
      : (portraitPoints, pixels)
    var json: [String: Any] = [
      "orientation": Turn(quarters: turn.quarters - natural).rawValue,
      "points": ["width": points.width, "height": points.height],
      "pixels": ["width": Int(upright.width), "height": Int(upright.height)],
    ]
    if let layout, let fold = layout.fold {
      let outer = layout.uprightPoints(fold.outer)
      let inner = layout.uprightPoints(fold.inner)
      json["fold"] = [
        "degrees": degrees,
        "display": (display == fold.inner.uniqueID ? FoldDisplay.inner : .outer).rawValue,
        "outer": ["width": outer.width, "height": outer.height],
        "inner": ["width": inner.width, "height": inner.height],
      ]
    }
    return json
  }

  // MARK: Pictures

  /// Follows the displays: each turn the lit one takes is the screen's, and
  /// a foldable's other display lit is the screen from then on. Follows the
  /// framebuffer.
  private func follow() {
    let displays = SimulatorDisplayCommands(simulator: simulator)
    let following = Task { [self] in
      for await configuration in displays.configurations() {
        guard case .settled = configuration.phase, let geometry = try? configuration.activeGeometry(),
          let turn = Turn(geometry.rotation)
        else {
          continue
        }
        var relit = false
        if layout?.fold != nil, let lit = try? configuration.activeDisplay(),
          lit.uniqueID != lock.withLock({ display })
        {
          relit = await light(lit)
        }
        turned(turn, relit: relit)
      }
    }
    lock.withLock { followingTurns = following }
    watch(lock.withLock { attachment })
  }

  /// Follows `attachment` while it is the lit display's: each new picture is
  /// drawn, encoded for each capture and recording; its end is the device's
  /// loss.
  private func watch(_ attachment: FramebufferAttachment) {
    Task { [self] in
      for await event in attachment.events {
        guard lock.withLock({ self.attachment === attachment }) else { return }
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
      if lock.withLock({ self.attachment === attachment }) { lost("its screen stopped") }
    }
  }

  /// The guest lit a foldable's `lit` display: the framebuffer moves to it,
  /// and the angle is the one its posture has unless the session moves the
  /// hinge. Says whether it moved.
  private func light(_ lit: SimulatorDisplay) async -> Bool {
    let attached: (FramebufferAttachment, IOSurface)
    do {
      attached = try await DeviceSession.framebuffer(simulator, on: .display(uniqueID: lit.uniqueID))
    } catch {
      log("\(simulator.name): the display it lit cannot be shown: \(describe(error))")
      return false
    }
    let (attachment, surface) = attached
    let (previous, waiting) = lock.withLock { () -> (FramebufferAttachment, [CheckedContinuation<Void, Never>]) in
      let previous = self.attachment
      self.attachment = attachment
      self.surface = surface
      display = lit.uniqueID
      natural = layout?.natural[lit.uniqueID] ?? 0
      portraitPoints = CGSize(
        width: Double(IOSurfaceGetWidth(surface)) / lit.scale, height: Double(IOSurfaceGetHeight(surface)) / lit.scale)
      if !folding, let fold = layout?.fold {
        degrees = lit.uniqueID == fold.inner.uniqueID ? FLAT_DEGREES : 0
      }
      let waiting = displayWaiters.filter { $0.value.0 == lit.uniqueID }
      for id in waiting.keys { displayWaiters[id] = nil }
      return (previous, waiting.values.map { $0.1 })
    }
    previous.cancel()
    watch(attachment)
    for waiter in waiting { waiter.resume() }
    return true
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
    let (encoders, taps) = lock.withLock { (Array(captures.values), Array(self.taps.values)) }
    for encoder in encoders { encoder.drawn() }
    for tap in taps { tap.drawn() }
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

  /// The display shows `turn`, or another display was `relit`: the
  /// pictures, the points the caller gives and the frame turn with it,
  /// demi-mobile hears of it, and those waiting for the orientation go on.
  private func turned(_ turn: Turn, relit: Bool = false) {
    let (changed, screen, waiting) = lock.withLock { () -> (Bool, [String: Any], [CheckedContinuation<Void, Never>]) in
      let changed = self.turn != turn || relit
      self.turn = turn
      let shown = Turn(quarters: turn.quarters - natural)
      let waiting = turnWaiters.filter { $0.value.0 == shown }
      for id in waiting.keys { turnWaiters[id] = nil }
      return (changed, screenJSONLocked(), waiting.values.map { $0.1 })
    }
    if changed {
      output.control(["event": "screen", "udid": udid, "screen": screen])
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
          if Turn(quarters: turn.quarters - natural) == target || Task.isCancelled { return true }
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
    let (turn, surface, screen, portraitPoints) = lock.withLock {
      (self.turn, self.surface, screenJSONLocked(), self.portraitPoints)
    }
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

  /// The point the HID takes for the upright point `x`, `y`. On the primary
  /// display, the HID takes the framebuffer's portrait points, whichever way
  /// the screen turns; on iPhone Duo's inner display it takes the upright
  /// points themselves, held either way, as it turns them by the display's
  /// own rotation (iOS 27.1: a tap at the portrait point of the Settings icon
  /// on the open device missed it, one at its upright point opened it).
  private func devicePoint(_ x: Double, _ y: Double) -> (Double, Double) {
    let (turn, points, primary) = lock.withLock { () -> (Turn, CGSize, Bool) in
      (self.turn, portraitPoints, layout?.fold.map { display == $0.outer.uniqueID } ?? true)
    }
    guard primary else { return (x, y) }
    let width = points.width
    let height = points.height
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
    let space = lock.withLock { readSpace }
    try await automation.setValue(text, for: .point(space?.read(CGPoint(x: x, y: y)) ?? CGPoint(x: x, y: y)))
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

  /// Turns off the keyboard's swipe-to-type introduction, which covers the
  /// keyboard the first time it shows, as a person does with a tap.
  private func skipKeyboardTutorials() async {
    do {
      try await simulator.preferences.apply(
        .preference(
          name: "DidShowContinuousPathIntroduction", value: "true", type: "bool",
          domain: "com.apple.keyboard.preferences"))
    } catch {
      log("the keyboard's introduction stays on: \(describe(error))")
    }
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

  // MARK: The hinge

  /// The file of a foldable's 3D body Xcode installs, by the device type's
  /// product class, as Device Hub draws it; none for a device that does not
  /// fold, or where this Xcode has no body for it.
  var body: String? {
    guard layout?.fold != nil else { return nil }
    let bundle = simulator.device.deviceType.bundle.bundleURL
    let profile = NSDictionary(contentsOf: bundle.appendingPathComponent("Contents/Resources/profile.plist"))
    guard let productClass = profile?["productClass"] as? String, let developer = Self.developerDirectory() else {
      return nil
    }
    let file = URL(fileURLWithPath: developer)
      .deletingLastPathComponent()
      .appendingPathComponent(
        "SharedFrameworks/DeviceKit.framework/Versions/A/PlugIns/CoreDevicePopDeviceKitExtension.devicekitplugin"
          + "/Contents/Resources/\(productClass).usdz")
    return FileManager.default.fileExists(atPath: file.path) ? file.path : nil
  }

  /// The Xcode developer directory `xcode-select` names, which simctl uses.
  private static func developerDirectory() -> String? {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/xcode-select")
    process.arguments = ["-p"]
    let pipe = Pipe()
    process.standardOutput = pipe
    process.standardError = FileHandle.nullDevice
    guard (try? process.run()) != nil else { return nil }
    process.waitUntilExit()
    let read = pipe.fileHandleForReading.readDataToEndOfFile()
    let path = String(decoding: read, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
    return process.terminationStatus == 0 && !path.isEmpty ? path : nil
  }

  /// Moves a foldable's hinge to `target` degrees, and returns once the
  /// display the guest lights for it shows, or once it is clear the guest
  /// kept the one it had.
  func fold(to target: Int) async throws -> [String: Any] {
    guard let fold = layout?.fold else { throw HelperError("\(simulator.name) does not fold") }
    let target = min(max(target, 0), FLAT_DEGREES)
    let opened = try lock.withLock { () -> Hinge in
      if let hinge { return hinge }
      let opened = try Hinge.open(simulator)
      hinge = opened
      return opened
    }
    let (from, lit) = lock.withLock { () -> (Int, String?) in
      folding = true
      defer { degrees = target }
      return (degrees, display)
    }
    defer { lock.withLock { folding = false } }
    await opened.sweep(from: from, to: target)
    let expected = Hinge.lit(after: target, from: from, lit: lit == fold.inner.uniqueID ? .inner : .outer)
    let id = expected == .inner ? fold.inner.uniqueID : fold.outer.uniqueID
    if id != lit {
      // The guest lights the inner display about half a second after the
      // hinge stops, and the outer one about three seconds after.
      if await displayLit(id, within: 6) {
        // The guest finishes moving its layout a little after the display
        // is lit: a hinge moved back within that time leaves the display
        // change waiting until the device is locked and woken (0 s did,
        // 0.3 s did not, iOS 27.1).
        try? await Task.sleep(for: .milliseconds(500))
      }
    }
    return screenJSON()
  }

  /// Returns once `id` is the lit display, or after `seconds`; says
  /// whether it is.
  private func displayLit(_ id: String, within seconds: Double) async -> Bool {
    await withTaskGroup(of: Void.self) { group in
      group.addTask { [self] in
        let waiter = UUID()
        await withTaskCancellationHandler {
          await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            let now = lock.withLock { () -> Bool in
              if display == id || Task.isCancelled { return true }
              displayWaiters[waiter] = (id, continuation)
              return false
            }
            if now { continuation.resume() }
          }
        } onCancel: { [self] in
          self.lock.withLock { self.displayWaiters.removeValue(forKey: waiter) }?.1.resume()
        }
      }
      group.addTask { try? await Task.sleep(for: .seconds(seconds)) }
      await group.next()
      group.cancelAll()
    }
    return lock.withLock { display == id }
  }

  /// The screen upright in points, and the lit display's own turn.
  var screenTurn: ScreenTurn {
    lock.withLock {
      ScreenTurn(
        upright: turn.landscape ? CGSize(width: portraitPoints.height, height: portraitPoints.width) : portraitPoints,
        natural: natural)
    }
  }

  /// The space the last tree read in, which a field's text is set in.
  func read(in space: AccessibilitySpace) {
    lock.withLock { readSpace = space }
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

  // MARK: Captures and the recordings

  func captureStart(capture: UInt32, width: Int, height: Int, fps: Int, bitrate: Int) throws {
    let encoder = try Encoder(
      capture: capture, width: width, height: height, fps: fps, bitrate: bitrate, session: self, output: output)
    lock.withLock { captures[capture] = encoder }
    encoder.keyFrame()
  }

  func captureKeyFrame(capture: UInt32) {
    lock.withLock { captures[capture] }?.keyFrame()
  }

  func captureRepeat(capture: UInt32) {
    lock.withLock { captures[capture] }?.repeatPicture()
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

  /// Hands demi-mobile every picture the device draws that differs from the
  /// one before, coded at the screen's own pixels, tagged `recording`; the
  /// current picture first.
  func recordStart(recording: UInt32, keyInterval: Int) {
    let tap = RecordingTap(recording: recording, session: self, output: output, keyInterval: keyInterval)
    lock.withLock { taps[recording] = tap }
    tap.drawn()
  }

  func recordStop(recording: UInt32) {
    lock.withLock { taps.removeValue(forKey: recording) }?.stop()
  }

  /// Lets the device go: its captures and recordings stop, the automation
  /// mode the bridge changed is restored, the HID and the framebuffer
  /// disconnect.
  func detach() async {
    let (encoders, taps, automation) = lock.withLock {
      () -> ([Encoder], [RecordingTap], AutomationBefore?) in
      detached = true
      defer {
        captures.removeAll()
        self.taps.removeAll()
      }
      return (Array(captures.values), Array(self.taps.values), automationBefore)
    }
    for encoder in encoders { encoder.stop() }
    for tap in taps { tap.stop() }
    if let automation {
      do {
        try await AutomationMode(simulator: simulator).restore(automation)
      } catch {
        log("\(simulator.name): the UI automation mode could not be restored: \(describe(error))")
      }
    }
    let (attachment, hinge) = lock.withLock { () -> (FramebufferAttachment, Hinge?) in
      followingTurns?.cancel()
      return (self.attachment, self.hinge)
    }
    hinge?.close()
    await hid.close()
    attachment.cancel()
  }
}
