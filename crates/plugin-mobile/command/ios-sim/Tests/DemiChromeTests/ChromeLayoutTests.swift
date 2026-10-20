// The frame's geometry against the numbers Apple's DeviceKit gives for the
// iPad Pro 13-inch's chrome (tablet5): DisplayViewModel's slice rectangles
// and DisplayChrome.frame's button rectangles, read by calling DeviceKit
// itself, but for the edges, which Demi draws under the corners and two
// points into them. Planted defects these catch: the pieces sized from
// chrome.json's sizing (46 points) instead of their artwork (96),
// unfloored thicknesses, DeviceKit's order or overlap, a button sized by
// its whole artwork, reversed offsets left as they are. Pure arithmetic,
// milliseconds.

import CoreGraphics
import Foundation
import Testing

@testable import DemiChrome

/// tablet5's chrome.json, with its sizing and padding, and `inputs`.
private func tablet(inputs: String = "[]") throws -> Chrome {
  let json = """
    {"images": {"topLeft": "iPadTL", "top": "iPadTop", "topRight": "iPadTR", "right": "iPadRight",
      "bottomRight": "iPadBR", "bottom": "iPadBase", "bottomLeft": "iPadBL", "left": "iPadLeft",
      "screen": "Screen", "sizing": {"leftWidth": 46, "rightWidth": 46, "topHeight": 46, "bottomHeight": 46},
      "padding": {"width": 10, "height": 9}, "devicePadding": {"top": 9, "left": 0, "bottom": 0, "right": 10}},
     "inputs": \(inputs)}
    """
  return try JSONDecoder().decode(Chrome.self, from: Data(json.utf8))
}

/// tablet5's artwork sizes; `Button` stands for a button 25 by 50 points.
private func artworkSize(_ name: String) -> CGSize? {
  switch name {
  case "iPadTL", "iPadTR", "iPadBL", "iPadBR": return CGSize(width: 96, height: 96)
  case "iPadTop", "iPadBase": return CGSize(width: 2, height: 96)
  case "iPadLeft", "iPadRight": return CGSize(width: 96, height: 2)
  case "Screen": return CGSize(width: 1, height: 1)
  case "Button": return CGSize(width: 25, height: 50)
  default: return nil
  }
}

private func expect(_ rect: CGRect, _ x: Double, _ y: Double, _ width: Double, _ height: Double, _ what: String) {
  let close = [rect.minX - x, rect.minY - y, rect.width - width, rect.height - height].allSatisfy { abs($0) < 1e-9 }
  #expect(close, "\(what) is \(rect), DeviceKit's is (\(x), \(y), \(width), \(height))")
}

@Test func theIPadsPiecesAreWhereDeviceKitPutsThem() throws {
  // An iPad Pro 13-inch, 2064 by 2752 pixels at scale 2, at render scale .37.
  let chrome = try tablet()
  let screen = CGSize(width: 1032, height: 1376)
  let fit = ChromeLayout.fit(chrome: chrome, screenPoints: screen, area: CGSize(width: 423.28, height: 549.82), quarterTurns: 0)
  #expect(abs(fit - 0.37) < 1e-12)
  let layout = try ChromeLayout(chrome: chrome, artworkSize: artworkSize, screenPoints: screen, scale: 0.37, quarterTurns: 0)
  expect(layout.full, 0, 0, 423.28, 549.82, "the full frame")
  expect(layout.chrome, 3.7, 3.33, 415.88, 543.16, "the outline")
  expect(layout.screen, 20.72, 20.35, 381.84, 509.12, "the screen")
  // The corners are drawn last, over the edges' ends, which reach two
  // points under them where DeviceKit's reach one (`crates/plugin-mobile/command/src/lib.rs`).
  #expect(layout.pieces.map(\.image) == [
    "Screen", "iPadTop", "iPadLeft", "iPadRight", "iPadBase", "iPadTL", "iPadTR", "iPadBL", "iPadBR",
  ])
  let pieces = layout.pieces.map(\.rect)
  expect(pieces[0], 20.72, 20.35, 381.84, 509.12, "the screen's artwork")
  expect(pieces[1], 36.7, 3.33, 349.88, 35, "the top edge")
  expect(pieces[2], 3.7, 36.33, 35, 477.16, "the left edge")
  expect(pieces[3], 384.58, 36.33, 35, 477.16, "the right edge")
  expect(pieces[4], 36.7, 511.49, 349.88, 35, "the bottom edge")
  expect(pieces[5], 3.7, 3.33, 35, 35, "the top left corner")
  expect(pieces[6], 384.58, 3.33, 35, 35, "the top right corner")
  expect(pieces[7], 3.7, 511.49, 35, 35, "the bottom left corner")
  expect(pieces[8], 384.58, 511.49, 35, 35, "the bottom right corner")
  #expect(layout.transform == .identity)
  expect(CGRect(origin: .zero, size: layout.size), 0, 0, 423.28, 549.82, "the canvas")
}

@Test func buttonsAreWhereDeviceKitPutsThem() throws {
  // DisplayChrome.frame for a 20 by 50 point button (25 wide less its 5
  // points of travel), offsets normal (-8, 100) and rollover (-3, 100), on
  // a screen of 2064 by 2752 points at .37: at each anchor and alignment.
  // Left of the outline those offsets are the wrong way round and swap.
  let anchors = ["top", "left", "right", "bottom"]
  let aligns = ["leading", "center", "trailing"]
  let inputs = anchors.flatMap { anchor in
    aligns.map { align in
      """
      {"image": "Button", "anchor": "\(anchor)", "align": "\(align)", "onTop": false,
       "offsets": {"normal": {"x": -8, "y": 100}, "rollover": {"x": -3, "y": 100}}}
      """
    }
  }
  let chrome = try tablet(inputs: "[\(inputs.joined(separator: ","))]")
  let layout = try ChromeLayout(
    chrome: chrome, artworkSize: artworkSize, screenPoints: CGSize(width: 2064, height: 2752), scale: 0.37,
    quarterTurns: 0)
  // (normal x, normal y, rollover x, rollover y), each 7.4 by 18.5.
  let deviceKit: [(Double, Double, Double, Double)] = [
    (0.74, 21.83, 2.59, 21.83), (395.9, 21.83, 397.75, 21.83), (791.06, 21.83, 792.91, 21.83),
    (-4.81, 40.33, -6.66, 40.33), (-4.81, 557.22, -6.66, 557.22), (-4.81, 1074.11, -6.66, 1074.11),
    (798.46, 40.33, 800.31, 40.33), (798.46, 557.22, 800.31, 557.22), (798.46, 1074.11, 800.31, 1074.11),
    (0.74, 1092.61, 2.59, 1092.61), (395.9, 1092.61, 397.75, 1092.61), (791.06, 1092.61, 792.91, 1092.61),
  ]
  #expect(layout.buttons.count == deviceKit.count)
  for (index, (button, expected)) in zip(layout.buttons, deviceKit).enumerated() {
    let place = "\(anchors[index / 3]) \(aligns[index % 3])"
    expect(button.normal, expected.0, expected.1, 7.4, 18.5, "the \(place) button")
    expect(button.rollover, expected.2, expected.3, 7.4, 18.5, "the \(place) button on hover")
  }
}

@Test func aTurnedFrameIsTurnedAboutItsCentre() throws {
  // The iPad at .5, a quarter turn clockwise (landscape right): the full
  // frame, 572 by 743 points upright, becomes 743 by 572, and the screen,
  // 9 + 46 points below the top, is that far left of the right side.
  let chrome = try tablet()
  let screen = CGSize(width: 1032, height: 1376)
  #expect(ChromeLayout.fit(chrome: chrome, screenPoints: screen, area: CGSize(width: 743, height: 572), quarterTurns: 1) == 0.5)
  let layout = try ChromeLayout(chrome: chrome, artworkSize: artworkSize, screenPoints: screen, scale: 0.5, quarterTurns: 1)
  expect(CGRect(origin: .zero, size: layout.size), 0, 0, 743, 572, "the canvas")
  expect(layout.screen.applying(layout.transform), 27.5, 28, 688, 516, "the turned screen")
  // The top left corner goes to the top right.
  expect(layout.pieces[5].rect.applying(layout.transform), 743 - 4.5 - 48, 5, 48, 48, "the turned top left corner")
}

@Test func theScreensCornerFollowsTheMasksEdge() throws {
  // A mask whose corners are flatter than a circle, as Apple's are: each
  // reaches 60 points along its edges on a superellipse of power 4, which
  // lies about 4 points further out on the diagonal than the circle of the
  // same open area. Drawn into a screen of the same size, every point of
  // the corner lies on the mask's edge within a third of a pixel, so the
  // picture's clip and the frame's hole, cut with the mask, are one shape.
  // Milliseconds.
  let reach = 60.0
  let power = 4.0
  let box = CGRect(x: 0, y: 0, width: 300, height: 600)
  // The edge's distance from the corner's centre at `angle` from level.
  let edge = { (angle: Double) in reach / pow(pow(abs(cos(angle)), power) + pow(abs(sin(angle)), power), 1 / power) }
  let url = FileManager.default.temporaryDirectory.appendingPathComponent("mask-\(UUID().uuidString).pdf")
  defer { try? FileManager.default.removeItem(at: url) }
  var media = box
  let pdf = try #require(CGContext(url as CFURL, mediaBox: &media, nil))
  pdf.beginPDFPage(nil)
  // Each corner's arc about its centre, a quarter turn of it per corner.
  let centres = [
    CGPoint(x: box.maxX - reach, y: box.minY + reach), CGPoint(x: box.maxX - reach, y: box.maxY - reach),
    CGPoint(x: box.minX + reach, y: box.maxY - reach), CGPoint(x: box.minX + reach, y: box.minY + reach),
  ]
  for (index, centre) in centres.enumerated() {
    for step in 0...400 {
      let angle = (Double(index) - 1 + Double(step) / 400) * .pi / 2
      let point = CGPoint(x: centre.x + edge(angle) * cos(angle), y: centre.y + edge(angle) * sin(angle))
      if index == 0 && step == 0 { pdf.move(to: point) } else { pdf.addLine(to: point) }
    }
  }
  pdf.closePath()
  pdf.fillPath()
  pdf.endPDFPage()
  pdf.closePDF()
  let mask = try #require(ScreenMask(url))
  let corner = mask.corner(pixels: box.size, screenPoints: box.size)
  try #require(corner.count >= 8)
  #expect(corner.first!.x == 0 && corner.first!.y > 45, "the corner leaves the upright edge at \(corner.first!)")
  #expect(corner.last!.y == 0 && corner.last!.x > 45, "the corner meets the level edge at \(corner.last!)")
  for point in corner {
    let across = reach - point.x
    let down = reach - point.y
    let off = hypot(across, down) - edge(atan2(down, across))
    #expect(abs(off) < 0.34, "\(point) lies \(off) pixels off the mask's edge")
  }
}

@Test func theSilhouetteIsWhatTheOpaqueFrameEncloses() {
  // A 12 by 12 frame: a soft outer ring (alpha 128) around an opaque ring
  // two pixels wide, around a hole whose own edge is soft too. The picture
  // may lie in the hole and under the opaque ring, never under the soft
  // outer pixels, through which it would show. Microseconds.
  let side = 12
  var alpha = [UInt8](repeating: 0, count: side * side)
  for y in 0..<side {
    for x in 0..<side {
      let ring = min(x, y, side - 1 - x, side - 1 - y)
      alpha[y * side + x] = ring == 0 ? 0 : ring == 1 ? 128 : ring <= 3 ? 255 : ring == 4 ? 100 : 0
    }
  }
  let shape = silhouette(alpha: alpha, width: side, height: side)
  for y in 0..<side {
    for x in 0..<side {
      let ring = min(x, y, side - 1 - x, side - 1 - y)
      #expect(shape[y * side + x] == (ring >= 2 ? 255 : 0), "pixel \(x), \(y) of ring \(ring)")
    }
  }
}
