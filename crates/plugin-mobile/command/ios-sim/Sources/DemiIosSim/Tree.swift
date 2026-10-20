// `inspect`'s tree: the accessibility read of the frontmost application, alerts
// and the keyboard included, with the content of the web views it holds found
// by a grid of hit-tests.
//
// - A bar the read lists without children, as iOS 26 lists its toolbars, is
//   filled the same way.
// - Safari's page the read cannot reach. There the guest bridge reads it, and
//   its web content takes the place of the page area
//   (`crates/plugin-mobile/command/src/ios/mod.rs`).

import FBControlCore
import FBSimulatorAX
import FBSimulatorControl
import Foundation

struct Tree {
  static let safari = "com.apple.mobilesafari"

  let session: DeviceSession

  func read() async throws -> [[String: Any]] {
    let simulator = session.simulator
    let native = try simulator.uiAutomation(backend: .accessibility)
    var keys = AXKeys.defaultSet
    keys.insert(.focused)
    keys.insert(.placeholder)
    let options = AccessibilityRequestOptions(
      format: .complete, keys: keys, remoteContentOptions: AccessibilityRemoteContentOptions())
    let read = try await native.describe(.frontmost, options: options)
    var elements = try Self.elements(of: read)
    let point = AccessibilityRequestOptions(format: .complete, keys: keys)
    try await Self.fillChildless(elements) { at in
      do {
        return try Self.elements(of: await native.describe(.point(at), options: point)).first
      } catch AccessibilityError.noTranslationObject(.point) {
        // Nothing answers there, as in a toolbar's gaps.
        return nil
      }
    }
    if !Self.contains(elements, where: { $0.traits.contains("WebContent") }),
      let safari = try? await simulator.application.processID(forBundleID: Self.safari),
      elements.first?.pid == safari
    {
      await session.beforeBridge()
      let bridge = try simulator.uiAutomation(
        backend: .axBridge(persistence: .exclusive, frontmostMethod: .windowServer, automationMode: true))
      let page = try await bridge.describe(.frontmost, options: AccessibilityRequestOptions(format: .complete, keys: keys))
      let content = Self.webContent(try Self.elements(of: page))
      if !content.isEmpty {
        elements = Self.graft(content, into: elements)
      }
    }
    return elements.map { $0.json }
  }

  /// An element as the read serializes it.
  // A node is built by one task and changed only once the reads that
  // built it ended, so it crosses from the hit-tests' tasks safely.
  final class Node: @unchecked Sendable {
    let type: String
    let role: String
    let label: String?
    let value: String?
    let identifier: String?
    let frame: CGRect
    let enabled: Bool?
    let focused: Bool?
    let traits: [String]
    let pid: Int32?
    var children: [Node]

    init(_ object: [String: Any]) {
      type = object["type"] as? String ?? ""
      let description = object["role_description"] as? String
      role = (description?.isEmpty == false ? description : nil) ?? type
      label = object["label"] as? String
      // An empty field's value reads as its placeholder, which is no text.
      let read = (object["value"] as? String) ?? (object["value"] as? NSNumber)?.stringValue
      value = read == (object["placeholder"] as? String) ? nil : read
      identifier = object["identifier"] as? String
      let frame = object["frame"] as? [String: Any] ?? [:]
      func number(_ key: String) -> Double { (frame[key] as? NSNumber)?.doubleValue ?? 0 }
      self.frame = CGRect(x: number("x"), y: number("y"), width: number("width"), height: number("height"))
      enabled = object["enabled"] as? Bool
      focused = object["focused"] as? Bool
      traits = object["traits"] as? [String] ?? []
      pid = (object["pid"] as? NSNumber)?.int32Value
      children = (object["children"] as? [[String: Any]] ?? []).map(Node.init)
    }

    var json: [String: Any] {
      var json: [String: Any] = [
        "role": role,
        "frame": ["x": frame.minX, "y": frame.minY, "width": frame.width, "height": frame.height],
        "secure": traits.contains("SecureTextField"),
        "keyboardKey": traits.contains("KeyboardKey"),
        "children": children.map(\.json),
      ]
      if let label { json["label"] = label }
      if let value { json["value"] = value }
      if let identifier { json["identifier"] = identifier }
      if let enabled { json["enabled"] = enabled }
      if traits.contains("Selected") { json["selected"] = true }
      if let focused { json["focused"] = focused }
      return json
    }
  }

  static func elements(of response: AccessibilityElementsResponse) throws -> [Node] {
    let data = try response.formattedOutputJSON(format: .complete)
    let document = try JSONSerialization.jsonObject(with: data) as? [String: Any]
    return (document?["elements"] as? [[String: Any]] ?? []).map(Node.init)
  }

  /// The distance between the points a bar is hit-tested at: less than the
  /// 44 points a control answers a hit-test over at the least, so a point
  /// lands in each. A smaller element within another, such as a field's
  /// dictation button, may be missed; the field is not. A bar costs 2-3 ms
  /// a point: Settings' toolbar 20 points, 35 ms; Files' navigation and tab
  /// bars 60, 140 ms (iOS 26.5).
  static let step: CGFloat = 40
  /// The most points one read hit-tests, about 0.3 s; a screen with more
  /// leaves the rest of its bars, in the tree's order, as the read lists
  /// them.
  static let maxPoints = 120
  /// The hit-tests under way at once: the simulator answers 16 at a time
  /// about six times as fast as one at a time.
  static let concurrent = 16

  /// Gives each bar the read lists without children the elements a hit-test
  /// finds in it.
  ///
  /// iOS 26 lists its toolbars so, Settings' search field and its buttons
  /// included, which a hit-test finds. idb's own grid keeps only another
  /// process's elements, such as a web view's.
  ///
  /// - A bar is a group at least twice as wide as it is tall, within the
  ///   application's frame. A larger group without children, such as Calendar's
  ///   day, would take seconds and is left as it is.
  /// - Every point of a bar's grid is tested, one within an element of the tree
  ///   too: a toolbar floats over the rows that scroll under it, and an element
  ///   found may hold another, such as a field's Clear button.
  /// - An element is adopted once, and not when the tree has it already, the
  ///   found elements' own subtrees included.
  /// - It becomes the bar's child in reading order, top to bottom, left to
  ///   right.
  static func fillChildless(
    _ elements: [Node], hitTest: @escaping @Sendable (CGPoint) async throws -> Node?
  ) async throws {
    guard let screen = elements.first?.frame, !screen.isEmpty else { return }
    var bars: [(group: Node, area: CGRect)] = []
    var known = Set<Identity>()
    func walk(_ node: Node) {
      known.insert(Identity(node))
      let area = node.frame.intersection(screen)
      if node.type == "Group", node.children.isEmpty, !area.isEmpty, area.width >= 2 * area.height {
        bars.append((node, area))
      }
      node.children.forEach(walk)
    }
    elements.forEach(walk)
    var points: [(bar: Int, point: CGPoint)] = []
    grid: for (index, bar) in bars.enumerated() {
      for y in stride(from: bar.area.minY + step / 2, to: bar.area.maxY, by: step) {
        for x in stride(from: bar.area.minX + step / 2, to: bar.area.maxX, by: step) {
          if points.count == maxPoints { break grid }
          points.append((index, CGPoint(x: x, y: y)))
        }
      }
    }
    // Tested at once, taken in the grid's order.
    var hits = [Node?](repeating: nil, count: points.count)
    try await withThrowingTaskGroup(of: (Int, Node?).self) { group in
      var next = 0
      func start() {
        guard next < points.count else { return }
        let (index, point) = (next, points[next].point)
        group.addTask { (index, try await hitTest(point)) }
        next += 1
      }
      for _ in 0..<concurrent { start() }
      while let (index, hit) = try await group.next() {
        hits[index] = hit
        start()
      }
    }
    var found = [[Node]](repeating: [], count: bars.count)
    var adopted: [Node] = []
    for ((index, _), hit) in zip(points, hits) {
      let (group, area) = bars[index]
      guard let hit, hit.pid == group.pid, area.contains(CGPoint(x: hit.frame.midX, y: hit.frame.midY)),
        !known.contains(Identity(hit)),
        // One that moves reads at a new frame each time.
        !adopted.contains(where: { Identity(sameElementAs: $0) == Identity(sameElementAs: hit) && $0.frame.intersects(hit.frame) })
      else { continue }
      func keepNew(_ node: Node) {
        known.insert(Identity(node))
        node.children = node.children.filter { !known.contains(Identity($0)) }
        node.children.forEach(keepNew)
      }
      keepNew(hit)
      adopted.append(hit)
      found[index].append(hit)
    }
    for (index, bar) in bars.enumerated() {
      bar.group.children = found[index].sorted { ($0.frame.minY, $0.frame.minX) < ($1.frame.minY, $1.frame.minX) }
    }
  }

  /// What tells one element from another across reads, which share no
  /// handle: its kind, process, label, identifier and, unless left out, its
  /// frame to the point.
  struct Identity: Hashable {
    let type: String
    let pid: Int32?
    let label: String?
    let identifier: String?
    let frame: [Int]

    init(_ node: Node) {
      self.init(sameElementAs: node, frame: node.frame)
    }

    init(sameElementAs node: Node, frame: CGRect = .null) {
      type = node.type
      pid = node.pid
      label = node.label
      identifier = node.identifier
      self.frame = frame.isNull ? [] : [frame.minX, frame.minY, frame.width, frame.height].map { Int($0.rounded()) }
    }
  }

  static func contains(_ nodes: [Node], where test: (Node) -> Bool) -> Bool {
    nodes.contains { test($0) || contains($0.children, where: test) }
  }

  /// The bridge's web content, its wrappers left out: each element with
  /// the WebContent trait and a label or a value, flattened in order.
  static func webContent(_ nodes: [Node]) -> [Node] {
    var found: [Node] = []
    func walk(_ node: Node) {
      if node.traits.contains("WebContent"), node.label?.isEmpty == false || node.value?.isEmpty == false {
        node.children = []
        found.append(node)
      }
      for child in node.children { walk(child) }
    }
    nodes.forEach(walk)
    return found
  }

  /// `content` in place of the page: the application's elements whose
  /// frames lie within the content's bounds give way to it.
  static func graft(_ content: [Node], into elements: [Node]) -> [Node] {
    guard let application = elements.first else { return content }
    let page = content.dropFirst().reduce(content[0].frame) { $0.union($1.frame) }
    application.children = application.children.filter { !page.contains($0.frame) }
    let at = application.children.firstIndex { $0.frame.minY >= page.minY } ?? application.children.count
    application.children.insert(contentsOf: content, at: at)
    return elements
  }
}
