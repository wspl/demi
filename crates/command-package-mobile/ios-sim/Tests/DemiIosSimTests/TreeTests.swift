// `inspect`'s tree where idb's read lists a bar without children: iOS 26's
// toolbar, which holds Settings' search field (`mobile.md` § iOS).

import CoreGraphics
import XCTest

@testable import DemiIosSim

final class TreeTests: XCTestCase {
  /// An element as idb serializes it.
  func element(
    _ type: String, _ label: String?, _ frame: CGRect, pid: Int32 = 7, children: [[String: Any]] = []
  ) -> [String: Any] {
    var object: [String: Any] = [
      "type": type, "pid": pid, "children": children,
      "frame": ["x": frame.minX, "y": frame.minY, "width": frame.width, "height": frame.height],
    ]
    if let label { object["label"] = label }
    return object
  }

  /// Settings' search field focused above the keyboard, as iOS 26.5 reads
  /// it, with a header bar and a page above. The read lists three groups
  /// without children: the header, whose search field fills it exactly
  /// and whose Add button the read lists already; the page, which is no
  /// bar; and the toolbar, over a row that scrolled under it, whose field
  /// answers with its magnifier and Clear button within it, beside the
  /// close button and over the keyboard's suggestions, which are another
  /// process's. The buttons answer a hit-test some way around their frames,
  /// the magnifier before the field does. Planted defects this catches: a
  /// bar left empty; the field dropped for its container's frame; Add,
  /// Clear or the magnifier adopted twice; the suggestions adopted; the page
  /// hit-tested; the points over the row skipped. Milliseconds.
  func testABarListedEmptyHoldsTheElementsAHitTestFindsOnce() async throws {
    let header = CGRect(x: 0, y: 62, width: 402, height: 44)
    let add = CGRect(x: 350, y: 62, width: 44, height: 44)
    let page = CGRect(x: 0, y: 124, width: 402, height: 340)
    let toolbar = CGRect(x: 0, y: 469, width: 402, height: 70)
    let field = CGRect(x: 17, y: 484, width: 308, height: 38)
    let magnifier = CGRect(x: 30, y: 493, width: 20, height: 19)
    let clear = CGRect(x: 291, y: 492, width: 17, height: 22)
    let close = CGRect(x: 347, y: 484, width: 38, height: 38)
    let suggestions = CGRect(x: 0, y: 515, width: 402, height: 24)
    let application = Tree.Node(
      element(
        "Application", "Settings", CGRect(x: 0, y: 0, width: 402, height: 874),
        children: [
          element("Group", "Header", header),
          element("Button", "Add", add),
          element("Group", nil, page),
          element("Button", "Privacy & Security", CGRect(x: 0, y: 470, width: 402, height: 61)),
          element("Group", "Toolbar", toolbar),
          element("Button", "Q", CGRect(x: 4, y: 590, width: 39, height: 54), pid: 9),
        ]))
    let tested = Tested()
    try await Tree.fillChildless([application]) { point in
      tested.add(point)
      let answers = { (frame: CGRect) in frame.insetBy(dx: -10, dy: -10).contains(point) }
      if header.contains(point) {
        return Tree.Node(self.element(point.x >= add.minX ? "Button" : "TextField", point.x >= add.minX ? "Add" : "Find", point.x >= add.minX ? add : header))
      }
      if suggestions.contains(point) { return Tree.Node(self.element("Group", "Suggestions", suggestions, pid: 9)) }
      if answers(magnifier) { return Tree.Node(self.element("Image", "Magnifier", magnifier)) }
      if answers(clear) { return Tree.Node(self.element("Button", "Clear", clear)) }
      if field.contains(point) {
        return Tree.Node(
          self.element(
            "TextField", nil, field,
            children: [self.element("Image", "Magnifier", magnifier), self.element("Button", "Clear", clear)]))
      }
      if answers(close) { return Tree.Node(self.element("Button", "Close", close)) }
      return nil
    }

    func shape(_ node: Tree.Node) -> String {
      node.children.isEmpty
        ? "\(node.type) \(node.label ?? "")" : "\(node.type) \(node.label ?? "") [\(node.children.map(shape).joined(separator: ", "))]"
    }
    XCTAssertEqual(
      application.children.map(shape),
      [
        "Group Header [TextField Find]",
        "Button Add",
        "Group ",
        "Button Privacy & Security",
        "Group Toolbar [TextField  [Button Clear], Button Close, Image Magnifier]",
        "Button Q",
      ])
    XCTAssertTrue(tested.points.allSatisfy { header.contains($0) || toolbar.contains($0) })
  }
}

/// The points hit-tested, from tasks of their own.
final class Tested: @unchecked Sendable {
  private let lock = NSLock()
  private var all: [CGPoint] = []
  func add(_ point: CGPoint) { lock.withLock { all.append(point) } }
  var points: [CGPoint] { lock.withLock { all } }
}
