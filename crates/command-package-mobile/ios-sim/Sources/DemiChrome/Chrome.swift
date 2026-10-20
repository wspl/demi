// A device's frame as DeviceKit's own renderer draws it, the one DeviceHub and
// Simulator.app show (`crates/command-package-mobile/src/live.rs`).
//
// The device type's chrome is a bundle in /Library/Developer/DeviceKit/Chrome:
// the bezel as PDF pieces, nine that stretch around the screen or one composite
// of them, and its buttons with the edge and the place each sits at.
//
// The geometry follows DeviceKit's DisplayChrome and DisplayViewModel, in the
// frame's points:
//
// - The screen in points is the pixel size divided by the device's scale,
//   truncated.
// - Every length is that times the render scale.
// - Each piece takes its own artwork's size, scaled and floored. The edges
//   reach two points under the corners at each end (DeviceKit's one point
//   leaves an edge's fading end showing at 1x).
// - The pieces are drawn into one context with no piece clipped: the screen,
//   the edges, then the corners, which DeviceKit draws before the edges
//   (`crates/command-package-mobile/src/lib.rs`).
// - The buttons sit outside the frame by their artwork's size less their
//   travel.
//
// Everything is laid out upright, then turned about the frame's centre.

import AppKit
import CoreGraphics
import Foundation

/// A device type's chrome as its `chrome.json` describes it.
public struct Chrome: Decodable {
  public struct Images: Decodable {
    let topLeft: String
    let top: String
    let topRight: String
    let right: String
    let bottomRight: String
    let bottom: String
    let bottomLeft: String
    let left: String
    let composite: String?
    let screen: String?
    let sizing: Sizing
    let padding: Padding
  }
  /// The bezel's widths around the screen, in the chrome's points.
  struct Sizing: Decodable {
    let leftWidth: Double
    let rightWidth: Double
    let topHeight: Double
    let bottomHeight: Double
  }
  /// The room around the bezel on each side, the same on opposite sides.
  struct Padding: Decodable {
    let width: Double
    let height: Double
  }
  struct Input: Decodable {
    enum Anchor: String, Decodable {
      case top, left, right, bottom
    }
    enum Align: String, Decodable {
      case leading, center, trailing
    }
    struct Point: Decodable {
      let x: Double
      let y: Double
    }
    /// Where the button sits at rest and where it stands out to on hover.
    struct Offsets: Decodable {
      let normal: Point
      let rollover: Point
    }
    let image: String
    let anchor: Anchor
    let align: Align
    let onTop: Bool
    let offsets: Offsets
  }
  let images: Images
  private let inputs: [Input]?

  /// The buttons; a chrome with none leaves the key out.
  var buttons: [Input] { inputs ?? [] }
}

/// The chrome's artwork, each PDF read once.
public final class ChromeArtwork {
  private let resources: URL
  private var images: [String: NSImage] = [:]

  public init(resources: URL) {
    self.resources = resources
  }

  public func image(_ name: String) -> NSImage? {
    if let image = images[name] { return image }
    let image = NSImage(contentsOf: resources.appendingPathComponent("\(name).pdf"))
    images[name] = image
    return image
  }

  /// The artwork's own size, in the chrome's points; nil when it is missing or empty.
  public func size(_ name: String) -> CGSize? {
    image(name).map(\.size).flatMap { $0.width > 0 && $0.height > 0 ? $0 : nil }
  }
}

public enum ChromeError: Error {
  case unreadable(String)
}

/// Where everything of a frame goes. Rectangles are upright, in the frame's
/// points; `transform` turns them and moves them onto the canvas.
public struct ChromeLayout {
  public struct Button {
    let image: String
    let onTop: Bool
    /// Where the button is drawn.
    let normal: CGRect
    /// Where it stands out to on hover, which the canvas makes room for.
    let rollover: CGRect
  }

  /// The device's outline (DeviceKit's chromeFrame).
  let chrome: CGRect
  /// The outline with the padding around it (fullFrame).
  let full: CGRect
  /// The screen (framebufferFrame).
  public let screen: CGRect
  /// The artwork in drawing order with its rectangle: the screen's, then
  /// the four edges, then the four corners; or the composite alone.
  let pieces: [(image: String, rect: CGRect)]
  let buttons: [Button]
  /// From the upright rectangles to the canvas: the turn about the full
  /// frame's centre, then the move that puts all of it at the origin.
  public let transform: CGAffineTransform
  /// The canvas, which holds the turned frame and its buttons.
  public let size: CGSize

  /// The screen in points DeviceKit lays the frame out for: the pixel size
  /// over the device's scale, truncated.
  static func displayPoints(_ points: CGSize) -> CGSize {
    CGSize(width: points.width.rounded(.towardZero), height: points.height.rounded(.towardZero))
  }

  /// The render scale that fits the full frame, turned, into `area`, as
  /// DeviceKit's fit does: never below a tenth, a half when the area has
  /// no size.
  public static func fit(chrome: Chrome, screenPoints: CGSize, area: CGSize, quarterTurns: Int) -> Double {
    let display = displayPoints(screenPoints)
    let images = chrome.images
    let width = display.width + images.sizing.leftWidth + images.sizing.rightWidth + 2 * images.padding.width
    let height = display.height + images.sizing.topHeight + images.sizing.bottomHeight + 2 * images.padding.height
    let turned = quarterTurns % 2 != 0
    let full = turned ? CGSize(width: height, height: width) : CGSize(width: width, height: height)
    guard area.width > 0, area.height > 0, full.width > 0, full.height > 0 else { return 0.5 }
    return max(0.1, min(area.width / full.width, area.height / full.height))
  }

  /// Lays the frame out around a screen of `screenPoints` (portrait) at
  /// render scale `scale`, turned `quarterTurns` clockwise.
  public init(
    chrome: Chrome, artworkSize: (String) -> CGSize?, screenPoints: CGSize, scale s: Double, quarterTurns: Int
  ) throws {
    let display = Self.displayPoints(screenPoints)
    let images = chrome.images
    let sizing = images.sizing
    let pad = CGPoint(x: s * images.padding.width, y: s * images.padding.height)
    let width = s * (display.width + sizing.leftWidth + sizing.rightWidth)
    let height = s * (display.height + sizing.topHeight + sizing.bottomHeight)
    let chromeFrame = CGRect(x: pad.x, y: pad.y, width: width, height: height)
    self.chrome = chromeFrame
    full = CGRect(x: 0, y: 0, width: width + 2 * pad.x, height: height + 2 * pad.y)
    screen = CGRect(
      x: s * (images.padding.width + sizing.leftWidth), y: s * (images.padding.height + sizing.topHeight),
      width: s * display.width, height: s * display.height)

    if let composite = images.composite, artworkSize(composite) != nil {
      pieces = [(composite, chromeFrame)]
    } else {
      func size(_ name: String) throws -> CGSize {
        guard let size = artworkSize(name) else { throw ChromeError.unreadable("the device's frame lacks \(name)") }
        return size
      }
      // Each piece's thickness is its artwork's, scaled and floored to
      // whole points, whatever the sizing says.
      let a = try (s * size(images.topLeft).width).rounded(.down)
      let b = try (s * size(images.topRight).width).rounded(.down)
      let c = try (s * size(images.topLeft).height).rounded(.down)
      let d = try (s * size(images.bottomLeft).height).rounded(.down)
      let l = try (s * size(images.left).width).rounded(.down)
      let r = try (s * size(images.right).width).rounded(.down)
      let t = try (s * size(images.top).height).rounded(.down)
      let u = try (s * size(images.bottom).height).rounded(.down)
      let W = width
      let H = height
      // Each edge reaches two points under the corners at either end, deep
      // enough at one pixel per point to hide its artwork's fading end.
      var placed: [(image: String, rect: CGRect)] = []
      if let screenImage = images.screen {
        placed.append((screenImage, screen))
      }
      // DeviceKit draws the corners first; an edge's last column of
      // artwork is half transparent and would lie over the corner's black
      // as a thin grey line, so the corners go last and cover it.
      placed += [
        (images.top, CGRect(x: a - 2, y: 0, width: W - a - b + 4, height: t)),
        (images.left, CGRect(x: 0, y: c - 2, width: l, height: H - c - d + 4)),
        (images.right, CGRect(x: W - r, y: c - 2, width: r, height: H - c - d + 4)),
        (images.bottom, CGRect(x: a - 2, y: H - d, width: W - a - b + 4, height: u)),
        (images.topLeft, CGRect(x: 0, y: 0, width: a, height: c)),
        (images.topRight, CGRect(x: W - b, y: 0, width: b, height: c)),
        (images.bottomLeft, CGRect(x: 0, y: H - d, width: a, height: d)),
        (images.bottomRight, CGRect(x: W - b, y: H - d, width: b, height: d)),
      ].map { ($0.0, $0.1.offsetBy(dx: pad.x, dy: pad.y)) }
      pieces = placed
    }

    buttons = chrome.buttons.compactMap { input in
      guard let artwork = artworkSize(input.image) else { return nil }
      return Self.button(input, artwork: artwork, chrome: chromeFrame, scale: s)
    }

    // The turn about the full frame's centre, exact for quarter turns,
    // then the move that brings the frame and every button's places,
    // turned, to the origin.
    let turns = ((quarterTurns % 4) + 4) % 4
    let (cos, sin): (Double, Double) = [(1, 0), (0, 1), (-1, 0), (0, -1)][turns]
    let centre = CGPoint(x: full.midX, y: full.midY)
    let turn = CGAffineTransform(translationX: -centre.x, y: -centre.y)
      .concatenating(CGAffineTransform(a: cos, b: sin, c: -sin, d: cos, tx: 0, ty: 0))
      .concatenating(CGAffineTransform(translationX: centre.x, y: centre.y))
    let union = buttons.reduce(full.applying(turn)) {
      $0.union($1.normal.applying(turn)).union($1.rollover.applying(turn))
    }
    transform = turn.concatenating(CGAffineTransform(translationX: -union.minX, y: -union.minY))
    size = union.size
  }

  /// A button's places, as DeviceKit's DisplayChrome.frame puts them: its
  /// artwork's size less its travel, at its anchor's edge outside the
  /// outline, aligned along it, moved by its offsets. Offsets that would
  /// move it inward on hover are the wrong way round and are swapped, as
  /// DeviceKit does.
  static func button(_ input: Chrome.Input, artwork: CGSize, chrome c: CGRect, scale s: Double) -> Button {
    var normal = input.offsets.normal
    var rollover = input.offsets.rollover
    let reversed: Bool
    switch input.anchor {
    case .top: reversed = normal.y < rollover.y
    case .left: reversed = normal.x < rollover.x
    case .right: reversed = normal.x > rollover.x
    case .bottom: reversed = normal.y > rollover.y
    }
    if reversed { swap(&normal, &rollover) }
    let w = s * (artwork.width - abs(normal.x - rollover.x))
    let h = s * (artwork.height - abs(normal.y - rollover.y))
    func along(_ min: Double, _ mid: Double, _ max: Double, _ length: Double) -> Double {
      switch input.align {
      case .leading: return min
      case .center: return mid - length / 2
      case .trailing: return max - length
      }
    }
    let base: CGPoint
    switch input.anchor {
    case .top: base = CGPoint(x: along(c.minX, c.midX, c.maxX, w), y: c.minY - h)
    case .bottom: base = CGPoint(x: along(c.minX, c.midX, c.maxX, w), y: c.maxY)
    case .left: base = CGPoint(x: c.minX - w, y: along(c.minY, c.midY, c.maxY, h))
    case .right: base = CGPoint(x: c.maxX, y: along(c.minY, c.midY, c.maxY, h))
    }
    func rect(_ offset: Chrome.Input.Point) -> CGRect {
      CGRect(x: base.x + s * offset.x, y: base.y + s * offset.y, width: w, height: h)
    }
    return Button(image: input.image, onTop: input.onTop, normal: rect(normal), rollover: rect(rollover))
  }

  /// The canvas in pixels at `backing` pixels per point. A side a rounding
  /// error past a whole pixel keeps to it, so a frame fitted to the view's
  /// area is never a pixel larger than it.
  public func pixels(backing: Double) -> (width: Int, height: Int) {
    (Int((size.width * backing - 1e-6).rounded(.up)), Int((size.height * backing - 1e-6).rounded(.up)))
  }

  /// A context for the canvas at `backing` pixels per point, drawn in
  /// points with its origin at the top left, as DeviceHub draws on a
  /// screen of that scale.
  public func canvas(backing: Double) throws -> CGContext {
    let (width, height) = pixels(backing: backing)
    guard width > 0, height > 0,
      let context = CGContext(
        data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
        space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
    else { throw ChromeError.unreadable("the frame's canvas could not be made") }
    context.translateBy(x: 0, y: Double(height))
    context.scaleBy(x: backing, y: -backing)
    return context
  }

  /// Runs `body` with `context` as AppKit's current context, flipped as
  /// DeviceKit's view is, under the layout's turn.
  private func drawing(_ context: CGContext, _ body: () -> Void) {
    NSGraphicsContext.saveGraphicsState()
    defer { NSGraphicsContext.restoreGraphicsState() }
    context.saveGState()
    defer { context.restoreGState() }
    context.concatenate(transform)
    NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: true)
    body()
  }

  /// Draws the bezel: the pieces into the one context, unclipped, each
  /// stretched into its rectangle.
  public func drawFrame(_ artwork: ChromeArtwork, in context: CGContext) {
    drawing(context) {
      for piece in pieces {
        artwork.image(piece.image)?.draw(in: piece.rect)
      }
    }
  }

  /// Draws the buttons behind the bezel, or those on top of it.
  public func drawButtons(_ artwork: ChromeArtwork, onTop: Bool, in context: CGContext) {
    drawing(context) {
      for button in buttons where button.onTop == onTop {
        artwork.image(button.image)?.draw(in: button.normal)
      }
    }
  }

  /// Cuts the screen out of what is drawn, in the mask's shape, for the
  /// picture beneath; the whole rectangle without a mask.
  public func cutScreen(mask: ScreenMask?, screenPoints: CGSize, in context: CGContext) {
    context.saveGState()
    defer { context.restoreGState() }
    context.concatenate(transform)
    context.setBlendMode(.destinationOut)
    if let mask {
      mask.draw(in: screen, screenPoints: screenPoints, context)
    } else {
      context.fill(screen)
    }
  }
}

/// The most points a traced corner keeps.
private let largestCorner = 96

/// The device type's framebuffer mask, the screen's shape: a PDF's first page.
public struct ScreenMask {
  let page: CGPDFPage
  let size: CGSize

  public init?(_ url: URL) {
    guard let document = CGPDFDocument(url as CFURL), let page = document.page(at: 1) else { return nil }
    self.page = page
    size = page.getBoxRect(.mediaBox).size
  }

  /// Draws the mask stretched into `rect` of `context`, whose origin is at
  /// the top left, turned a quarter back when the mask is drawn the other
  /// way from the screen, as DeviceKit corrects it.
  func draw(in rect: CGRect, screenPoints: CGSize, _ context: CGContext) {
    guard size.width > 0, size.height > 0 else { return }
    let corrected = (size.width > size.height) != (screenPoints.width > screenPoints.height)
    let drawn = corrected ? CGSize(width: rect.height, height: rect.width) : rect.size
    context.saveGState()
    defer { context.restoreGState() }
    context.translateBy(x: rect.midX, y: rect.midY)
    // PDF's origin is at the bottom left.
    context.scaleBy(x: 1, y: -1)
    // A quarter turn counterclockwise, as the screen sees it.
    if corrected { context.rotate(by: .pi / 2) }
    context.translateBy(x: -drawn.width / 2, y: -drawn.height / 2)
    context.scaleBy(x: drawn.width / size.width, y: drawn.height / size.height)
    context.drawPDFPage(page)
  }

  /// The screen's top left corner in a screen of `pixels`, as the mask
  /// draws it: the line where the mask turns half opaque, from where it
  /// leaves the upright edge to where it meets the level one, in pixels from
  /// the corner. Empty where the mask leaves the corner square.
  ///
  /// The frame's hole is cut with this mask, so the picture's clip, drawn
  /// from these points, follows the hole exactly; a circle of any radius
  /// strays from the mask's smooth corner by pixels along the diagonal.
  public func corner(pixels: CGSize, screenPoints: CGSize) -> [CGPoint] {
    let side = Int(min(pixels.width, pixels.height) / 2)
    guard side > 1,
      let context = CGContext(
        data: nil, width: side, height: side, bitsPerComponent: 8, bytesPerRow: side,
        space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.alphaOnly.rawValue)
    else { return [] }
    // The context is the mask's top left quarter, origin at the top left.
    context.translateBy(x: 0, y: Double(side))
    context.scaleBy(x: 1, y: -1)
    draw(in: CGRect(origin: .zero, size: pixels), screenPoints: screenPoints, context)
    guard let data = context.data?.assumingMemoryBound(to: UInt8.self) else { return [] }
    let alpha = { (x: Int, y: Int) in Double(data[y * side + x]) / 255 }
    // Short of whole by more than the rounding of a straight edge.
    let opaque = 0.99
    // Where a line of pixels turns half opaque, between the centres of the
    // two pixels it turns between; nil if it never does.
    func crossing(_ value: (Int) -> Double) -> Double? {
      var previous = value(0)
      if previous >= 0.5 { return 1 - previous }
      for index in 1..<side {
        let next = value(index)
        if next >= 0.5 { return Double(index) - 0.5 + (0.5 - previous) / (next - previous) }
        previous = next
      }
      return nil
    }
    // The corner leaves the upright edge below the lowest row whose edge
    // pixel is not wholly opaque, and meets the level edge right of the
    // rightmost such column: a smooth corner meets its edges tangentially,
    // a fraction of a pixel in for many pixels.
    guard let lowest = (0..<side).last(where: { alpha(0, $0) < opaque }),
      let rightmost = (0..<side).last(where: { alpha($0, 0) < opaque }), lowest > 0, rightmost > 0
    else { return [] }
    let start = Double(lowest + 1)
    // The steep half read along rows, up from the upright edge; the level
    // half along columns, each where it crosses its line more squarely.
    var points = [CGPoint(x: 0, y: start)]
    for row in stride(from: lowest, through: 0, by: -1) {
      let y = Double(row) + 0.5
      guard let x = crossing({ alpha($0, row) }) else { continue }
      if x > y { break }
      points.append(CGPoint(x: x, y: y))
    }
    let across = points.last?.x ?? 0
    for column in 0..<side {
      let x = Double(column) + 0.5
      guard x > across, x < Double(rightmost + 1), let y = crossing({ alpha(column, $0) }), y < x else { continue }
      points.append(CGPoint(x: x, y: y))
    }
    points.append(CGPoint(x: Double(rightmost + 1), y: 0))
    // A few dozen points follow the curve to a fraction of a pixel.
    let step = max(1, (points.count + largestCorner - 1) / largestCorner)
    return points.enumerated().compactMap { index, point in
      index % step == 0 || index == points.count - 1 ? point : nil
    }
  }

}
