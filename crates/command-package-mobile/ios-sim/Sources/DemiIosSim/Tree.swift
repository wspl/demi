// `inspect`'s tree: the accessibility read of the frontmost application,
// alerts and the keyboard included, with the content of the web views it
// holds found by a grid of hit-tests. Safari's page the read cannot reach;
// there the guest bridge reads it, and its web content takes the place of
// the page area (`mobile.md` § iOS).

import FBControlCore
import FBSimulatorAX
import FBSimulatorControl
import Foundation

struct Tree {
  /// The simulator's UI automation mode, which the bridge turns on.
  static let automationKey = "AutomationEnabled"
  static let automationDomain = "com.apple.Accessibility"
  static let safari = "com.apple.mobilesafari"

  let session: DeviceSession

  func read() async throws -> [[String: Any]] {
    let simulator = session.simulator
    let native = try simulator.uiAutomation(backend: .accessibility)
    var keys = AXKeys.defaultSet
    keys.insert(.focused)
    let options = AccessibilityRequestOptions(
      format: .complete, keys: keys, remoteContentOptions: AccessibilityRemoteContentOptions())
    let read = try await native.describe(.frontmost, options: options)
    var elements = try Self.elements(of: read)
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
  final class Node {
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
      value = (object["value"] as? String) ?? (object["value"] as? NSNumber)?.stringValue
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
