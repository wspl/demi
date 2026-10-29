// One attached simulator: its framebuffer, its HID connection, the
// orientation its screen shows, its captures and its recordings.
//
// A foldable, as iPhone Duo, has two displays, of which the guest lights
// one: the outer display, the device's primary one, while closed, the inner
// one open. The session attaches the framebuffer of each, as Device Hub
// shows each on its own face, so a capture of either shows it the moment
// the guest draws on it. The lit display is the one the guest names: the
// display it switches to from the moment its display configuration names it
// incoming, about half a second into a close where the switch settles three
// seconds later. The screen, its points, the tree and screenshots are the lit
// display's, and the HID goes to it by itself. A display's own turn while
// the device is upright, a quarter turn for iPhone Duo's inner display, is
// taken off the orientation the screen reports, so an open device held
// upright is in portrait.
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
// it takes, whoever asked for it. How the device is held is apart from it:
// the orientation it was last turned to, which only a turn changes. The
// screenshots, the tree and the points the caller gives are the screen's;
// the live captures and the frame are the device's as it is held, so the
// view turns the device only when it is turned, and shows an app that did
// not follow on its side, as Simulator.app does.

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

  /// The layout of `configuration`'s displays on a device of the type in
  /// `bundle`. Each display's turn is the one its panel is mounted at, as the
  /// device type declares it to the guest, rather than the turns the displays
  /// report: those follow the device, and a closed foldable's outer display
  /// stays upright while its inner one turns, so their difference is the
  /// upright one only while the device is upright.
  init(_ configuration: SimulatorDisplayConfiguration, deviceType bundle: URL) {
    let integrated = configuration.displays.filter(\.isIntegrated)
    let primary = integrated.first(where: \.isPrimary)
    let mounted = Self.mountedDegrees(bundle)
    let base = primary.flatMap { mounted[$0.name] } ?? 0
    // The interface turns back what the panel's mounting turned: a panel
    // mounted at 270° shows it a quarter turn clockwise.
    natural = Dictionary(
      integrated.map { ($0.uniqueID, ((base - (mounted[$0.name] ?? 0)) / 90 % 4 + 4) % 4) },
      uniquingKeysWith: { first, _ in first })
    if integrated.count == 2, let primary, let inner = integrated.first(where: { !$0.isPrimary }) {
      fold = (primary, inner)
    } else {
      fold = nil
    }
  }

  /// The degrees each integrated display's panel is mounted at, by its
  /// name, from the device type's capabilities.
  private static func mountedDegrees(_ bundle: URL) -> [String: Int] {
    let file = bundle.appendingPathComponent("Contents/Resources/capabilities.plist")
    let capabilities = NSDictionary(contentsOf: file)?["capabilities"] as? [String: Any]
    let displays = capabilities?["displays"] as? [[String: Any]] ?? []
    return Dictionary(
      displays.compactMap { display in
        guard display["displayType"] as? String == "integrated", let name = display["displayName"] as? String,
          let degrees = (display["nativeRotation"] as? NSNumber)?.intValue
        else { return nil }
        return (name, degrees)
      },
      uniquingKeysWith: { first, _ in first })
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

/// One display's framebuffer, attached, and the turn its picture shows, as
/// its display last said.
private struct Face {
  var attachment: FramebufferAttachment
  var surface: IOSurface?
  var turn = Turn.portrait
}

final class DeviceSession: @unchecked Sendable {
  let simulator: Simulator
  let udid: String
  private let output: Output
  private let hid: SimulatorHIDSession
  private let layout: DisplayLayout?
  private let lock = NSLock()
  /// Each display's face, by its display's unique ID: a foldable's two, or
  /// the one display of a device that does not fold, under the ID of the lit
  /// display.
  private var faces: [String: Face]
  /// The lit display, by its unique ID; none for a device whose display
  /// idb does not name, whose face is under the empty ID.
  private var display: String?
  /// The screen in the points of its framebuffer, which stays upright
  /// whichever way the screen turns.
  private var portraitPoints: CGSize
  /// The quarter turns clockwise the lit display shows while the device is
  /// upright, which the orientation the screen reports leaves out.
  private var natural = 0
  /// How the device is held: the orientation it was last turned to, which
  /// the app on either display may have refused.
  private var held = Turn.portrait
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
  /// The display the guest's last settled configuration names active.
  private var settled: String?
  /// Those waiting for a display to be lit and settled, by their identity.
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
    lit: SimulatorDisplay?, faces: [String: FramebufferAttachment], portraitPoints: CGSize
  ) {
    self.simulator = simulator
    self.udid = simulator.udid
    self.output = output
    self.hid = hid
    self.layout = layout
    self.faces = faces.mapValues { Face(attachment: $0, surface: $0.initialSurface) }
    self.portraitPoints = portraitPoints
    self.display = lit?.uniqueID
    self.settled = lit?.uniqueID
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
    let layout = lit.map { _ in DisplayLayout(configuration, deviceType: simulator.device.deviceType.bundle.bundleURL) }
    var faces = [lit?.uniqueID ?? "": attachment]
    // A foldable's other display, which is off, shows on its face too.
    if let fold = layout?.fold {
      for other in [fold.outer, fold.inner] where faces[other.uniqueID] == nil {
        do {
          faces[other.uniqueID] = try await framebuffer(simulator, on: .display(uniqueID: other.uniqueID)).0
        } catch {
          log("\(simulator.name): its \(other.name) display cannot be shown: \(describe(error))")
        }
      }
    }
    let hid: SimulatorHIDSession
    do {
      hid = try await SimulatorHIDSession.open(simulator)
    } catch {
      for face in faces.values { face.cancel() }
      throw error
    }
    let session = DeviceSession(
      simulator: simulator, output: output, hid: hid, layout: layout, lit: lit, faces: faces, portraitPoints: points)
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

  /// A display's framebuffer, attached.
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
    let face = faces[display ?? ""]
    var json = Self.shownJSON(
      surface: face?.surface, turn: face?.turn ?? .portrait, natural: natural, points: portraitPoints)
    json["held"] = held.rawValue
    if let layout, let fold = layout.fold {
      let outer = layout.uprightPoints(fold.outer)
      let inner = layout.uprightPoints(fold.inner)
      var displays: [String: Any] = [:]
      for (name, each) in [(FoldDisplay.outer, fold.outer), (.inner, fold.inner)] {
        guard let face = faces[each.uniqueID] else { continue }
        let natural = layout.natural[each.uniqueID] ?? 0
        let points = face.surface.map {
          CGSize(width: Double(IOSurfaceGetWidth($0)) / each.scale, height: Double(IOSurfaceGetHeight($0)) / each.scale)
        } ?? .zero
        displays[name.rawValue] = Self.shownJSON(surface: face.surface, turn: face.turn, natural: natural, points: points)
      }
      json["fold"] = [
        "degrees": degrees,
        "display": (display == fold.inner.uniqueID ? FoldDisplay.inner : .outer).rawValue,
        "outer": ["width": outer.width, "height": outer.height],
        "inner": ["width": inner.width, "height": inner.height],
        "displays": displays,
      ]
    }
    return json
  }

  /// A display's picture as the screen says it, upright: its orientation,
  /// its points, from those of its framebuffer, and its pixels.
  private static func shownJSON(surface: IOSurface?, turn: Turn, natural: Int, points portrait: CGSize)
    -> [String: Any]
  {
    let pixels = surface.map { CGSize(width: IOSurfaceGetWidth($0), height: IOSurfaceGetHeight($0)) } ?? .zero
    let (points, upright) =
      turn.landscape
      ? (CGSize(width: portrait.height, height: portrait.width), CGSize(width: pixels.height, height: pixels.width))
      : (portrait, pixels)
    return [
      "orientation": Turn(quarters: turn.quarters - natural).rawValue,
      "points": ["width": points.width, "height": points.height],
      "pixels": ["width": Int(upright.width), "height": Int(upright.height)],
    ]
  }

  // MARK: Pictures

  /// Follows the displays: each turn the lit one takes is the screen's, and
  /// a foldable's other display lit is the screen from then on. Follows the
  /// framebuffer.
  private func follow() {
    let displays = SimulatorDisplayCommands(simulator: simulator)
    let following = Task { [self] in
      for await configuration in displays.configurations() {
        // A switch under way has settled on no display yet.
        if case .transitioning = configuration.phase { lock.withLock { settled = nil } }
        // The display a foldable switches to is lit from the moment the
        // guest names it: it draws on it already.
        if case let .transitioning(incoming?) = configuration.phase, layout?.fold != nil, incoming.isIntegrated,
          incoming.uniqueID != lock.withLock({ display }), light(incoming)
        {
          turned(lock.withLock { faces[incoming.uniqueID]?.turn } ?? .portrait, relit: true)
          continue
        }
        guard case .settled = configuration.phase, let geometry = try? configuration.activeGeometry(),
          let turn = Turn(geometry.rotation)
        else {
          continue
        }
        // Each other face shows its display turned as that display says;
        // the lit one's turn is `turned`'s, which tells of it.
        lock.withLock {
          for each in configuration.displays where each.uniqueID != display {
            if let turn = Turn(each.rotation), faces[each.uniqueID] != nil { faces[each.uniqueID]?.turn = turn }
          }
        }
        var relit = false
        if layout?.fold != nil, let lit = try? configuration.activeDisplay(),
          lit.uniqueID != lock.withLock({ display })
        {
          relit = light(lit)
        }
        turned(turn, relit: relit)
        settle(try? configuration.activeDisplay().uniqueID)
      }
    }
    let attachments = lock.withLock { () -> [(String, FramebufferAttachment)] in
      followingTurns = following
      return faces.map { ($0.key, $0.value.attachment) }
    }
    for (id, attachment) in attachments { watch(id, attachment) }
  }

  /// Follows the face of display `id`: each new picture is drawn, encoded
  /// for each of its captures, and the lit display's for each recording; its
  /// end is the device's loss.
  private func watch(_ id: String, _ attachment: FramebufferAttachment) {
    Task { [self] in
      for await event in attachment.events {
        switch event {
        case let .surfaceChanged(changed):
          lock.withLock { faces[id]?.surface = changed }
          drawn(id)
        case .frameRendered:
          drawn(id)
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

  /// The guest lit a foldable's `lit` display: the screen is its from now
  /// on, and the angle is the one its posture has unless the session moves
  /// the hinge. Says whether it is, which a display without a face is not.
  private func light(_ lit: SimulatorDisplay) -> Bool {
    let lit = lock.withLock { () -> Bool in
      guard let surface = faces[lit.uniqueID]?.surface else { return false }
      display = lit.uniqueID
      natural = layout?.natural[lit.uniqueID] ?? 0
      portraitPoints = CGSize(
        width: Double(IOSurfaceGetWidth(surface)) / lit.scale, height: Double(IOSurfaceGetHeight(surface)) / lit.scale)
      // A display lit while Demi does not move the hinge is a fold outside
      // Demi, at the angle that display has, unless the angle Demi left the
      // hinge at lights it too: the guest lights the outer display up to
      // 120° and the inner one from 20°, some seconds after the hinge
      // stopped.
      if !folding, let fold = layout?.fold {
        let inner = lit.uniqueID == fold.inner.uniqueID
        if inner ? degrees < 20 : degrees > 120 {
          degrees = inner ? FLAT_DEGREES : 0
        }
      }
      return true
    }
    if !lit { log("\(simulator.name): the display it lit cannot be shown") }
    return lit
  }

  /// The guest's display configuration settled with `active` active: those
  /// waiting for it go on.
  private func settle(_ active: String?) {
    let waiting = lock.withLock { () -> [CheckedContinuation<Void, Never>] in
      settled = active
      let waiting = displayWaiters.filter { $0.value.0 == active }
      for id in waiting.keys { displayWaiters[id] = nil }
      return waiting.values.map { $0.1 }
    }
    for waiter in waiting { waiter.resume() }
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

  /// The lit display's upright picture now, or that of a foldable's
  /// `display`; nil while it shows none.
  func upright(_ display: FoldDisplay? = nil) -> CIImage? {
    let face = lock.withLock { faces[faceID(display)] }
    guard let surface = face?.surface else { return nil }
    return CIImage(ioSurface: surface).oriented(face?.turn.picture ?? .up)
  }

  /// The lit display's picture now, or that of a foldable's `display`, as
  /// the device is held: what the live captures show; nil while it shows
  /// none.
  func asHeld(_ display: FoldDisplay? = nil) -> CIImage? {
    let (face, turn) = lock.withLock { () -> (Face?, Turn) in
      let id = faceID(display)
      return (faces[id], Turn(quarters: held.quarters + (layout?.natural[id] ?? 0)))
    }
    guard let surface = face?.surface else { return nil }
    return CIImage(ioSurface: surface).oriented(turn.picture)
  }

  /// The ID of the face of a foldable's `display`, or of the lit display's.
  private func faceID(_ display: FoldDisplay?) -> String {
    guard let display, let fold = layout?.fold else { return self.display ?? "" }
    return display == .inner ? fold.inner.uniqueID : fold.outer.uniqueID
  }

  /// Display `id` drew a picture: each capture showing it encodes it, and
  /// the lit display's goes to each recording too.
  private func drawn(_ id: String) {
    let (encoders, taps, lit) = lock.withLock { () -> ([Encoder], [RecordingTap], Bool) in
      let lit = id == display ?? ""
      let encoders = captures.values.filter { (($0.display.map { faceID($0) }) ?? (display ?? "")) == id }
      return (encoders, lit ? Array(self.taps.values) : [], lit)
    }
    for encoder in encoders { encoder.drawn() }
    for tap in taps { tap.drawn() }
    if lit { tellDrawn() }
  }

  /// The lit display's pictures: each capture of it and each recording draw
  /// the display now lit.
  private func drawnLit() {
    drawn(lock.withLock { display ?? "" })
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
  /// screenshots and the points the caller gives turn with it, the live
  /// captures show it as the device is held, demi-mobile hears of it, and
  /// those waiting for the orientation go on.
  private func turned(_ turn: Turn, relit: Bool = false) {
    let (changed, screen, waiting) = lock.withLock { () -> (Bool, [String: Any], [CheckedContinuation<Void, Never>]) in
      let id = display ?? ""
      let changed = faces[id]?.turn != turn || relit
      faces[id]?.turn = turn
      let shown = Turn(quarters: turn.quarters - natural)
      let waiting = turnWaiters.filter { $0.value.0 == shown }
      for id in waiting.keys { turnWaiters[id] = nil }
      return (changed, screenJSONLocked(), waiting.values.map { $0.1 })
    }
    if changed {
      output.control(["event": "screen", "udid": udid, "screen": screen])
      // The captures draw the display's new picture.
      drawnLit()
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
          let turn = faces[display ?? ""]?.turn ?? .portrait
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
    // The frame of the device as it is held, which the live captures show.
    let (turn, surface, screen, portraitPoints) = lock.withLock {
      (
        Turn(quarters: held.quarters + natural), faces[display ?? ""]?.surface, screenJSONLocked(),
        self.portraitPoints
      )
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
      let primary = layout?.fold.map { display == $0.outer.uniqueID } ?? true
      return (faces[display ?? ""]?.turn ?? .portrait, portraitPoints, primary)
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

  /// A viewer's finger, which a fold under way takes away: the point it
  /// was given is on a display that is going dark, so a touch that comes as
  /// the display changes is dropped, as a finger on a phone being folded
  /// reaches nothing, rather than failed back to the viewer.
  func touch(phase: String, x: Double, y: Double) async throws {
    let (px, py) = devicePoint(x, y)
    let event: SimulatorHIDEvent
    switch phase {
    // A finger that moves is a contact still down, at its new place.
    case "down", "move": event = .touch(direction: .down, x: px, y: py)
    case "up": event = .touch(direction: .up, x: px, y: py)
    default: throw HelperError("a touch is down, move or up, not \(phase)")
    }
    do {
      try await send(event)
    } catch SimulatorDisplayError.changed, SimulatorDisplayError.transitioning {
      log("\(simulator.name): a touch came as its display changed, and is dropped")
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
    // The captures show the device turned at once, whatever its displays do:
    // demi-mobile hears of it now, and lays them out anew.
    let (changed, screen, ids) = lock.withLock { () -> (Bool, [String: Any], [String]) in
      let changed = held != turn
      held = turn
      return (changed, screenJSONLocked(), Array(faces.keys))
    }
    if changed {
      output.control(["event": "screen", "udid": udid, "screen": screen])
      for id in ids { drawn(id) }
    }
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

  /// The hinge, opened at the first move.
  private func openedHinge() throws -> Hinge {
    try lock.withLock { () -> Hinge in
      if let hinge { return hinge }
      let opened = try Hinge.open(simulator, at: degrees)
      hinge = opened
      return opened
    }
  }

  /// Turns a foldable's hinge towards `target` degrees at once, from where
  /// it is, as a view's slider drags it; the display the guest lights then
  /// shows by itself.
  func hinge(to target: Int) throws {
    guard layout?.fold != nil else { throw HelperError("\(simulator.name) does not fold") }
    let opened = try openedHinge()
    let target = min(max(target, 0), FLAT_DEGREES)
    let (from, screen) = lock.withLock { () -> (Int, [String: Any]) in
      folding = true
      let from = degrees
      degrees = target
      return (from, screenJSONLocked())
    }
    opened.move(to: target, from: from)
    // The angle is the screen's, whichever display the guest lights.
    output.control(["event": "screen", "udid": udid, "screen": screen])
    Task { [self] in
      await opened.rest()
      lock.withLock { if !opened.moving { folding = false } }
    }
  }

  /// Moves a foldable's hinge to `target` degrees, and returns once the
  /// display the guest lights for it shows, or once it is clear the guest
  /// kept the one it had.
  func fold(to target: Int) async throws -> [String: Any] {
    guard let fold = layout?.fold else { throw HelperError("\(simulator.name) does not fold") }
    let target = min(max(target, 0), FLAT_DEGREES)
    let opened = try openedHinge()
    let (from, lit) = lock.withLock { () -> (Int, String?) in
      folding = true
      defer { degrees = target }
      return (degrees, display)
    }
    opened.move(to: target, from: from)
    await opened.rest()
    defer { lock.withLock { if !opened.moving { folding = false } } }
    let expected = Hinge.lit(after: target, from: from, lit: lit == fold.inner.uniqueID ? .inner : .outer)
    let id = expected == .inner ? fold.inner.uniqueID : fold.outer.uniqueID
    // A switch the view's hinge began is lit already, and still under way
    // until the guest settles on it.
    if lock.withLock({ settled }) != id {
      // The guest settles on the inner display about half a second after
      // the hinge stops, and on the outer one about three seconds after.
      if await displayLit(id, within: 6) {
        // The guest finishes moving its layout a little after it settled: a
        // hinge moved back within that time leaves the display change
        // waiting until the device is locked and woken (0 s did, 0.3 s did
        // not, iOS 27.1).
        try? await Task.sleep(for: .milliseconds(500))
      }
    }
    return screenJSON()
  }

  /// Returns once the guest settled with `id` active, or after `seconds`;
  /// says whether it did.
  private func displayLit(_ id: String, within seconds: Double) async -> Bool {
    await withTaskGroup(of: Void.self) { group in
      group.addTask { [self] in
        let waiter = UUID()
        await withTaskCancellationHandler {
          await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            let now = lock.withLock { () -> Bool in
              if settled == id || Task.isCancelled { return true }
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
    return lock.withLock { settled == id }
  }

  /// The screen upright in points, and the lit display's own turn.
  var screenTurn: ScreenTurn {
    lock.withLock {
      let turn = faces[display ?? ""]?.turn ?? .portrait
      return ScreenTurn(
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

  /// Starts capture `capture` of the lit display, or of a foldable's
  /// `display`.
  func captureStart(capture: UInt32, display: FoldDisplay?, width: Int, height: Int, fps: Int, bitrate: Int) throws {
    if display != nil, layout?.fold == nil { throw HelperError("\(simulator.name) does not fold") }
    let encoder = try Encoder(
      capture: capture, display: display, width: width, height: height, fps: fps, bitrate: bitrate, session: self,
      output: output)
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
    let (faces, hinge) = lock.withLock { () -> ([Face], Hinge?) in
      followingTurns?.cancel()
      return (Array(self.faces.values), self.hinge)
    }
    hinge?.close()
    await hid.close()
    for face in faces { face.attachment.cancel() }
  }
}
