// `frame`: the device's frame as Simulator.app draws it (`mobile.md` § The
// live view). The device type's profile names its chrome, a bundle in
// /Library/Developer/DeviceKit/Chrome: the bezel as PDF pieces, nine that
// stretch around the screen or one composite of them, and its buttons with
// the edge and the place each sits at. The screen's rounded corners are the
// device type's framebuffer mask. The frame is drawn once for the size the
// view asks for, turned as the picture is, with the screen cut out of it.

import CoreGraphics
import CoreSimulator
import FBSimulatorControl
import CoreImage
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// Where Xcode installs the device frames.
private let chromeRoot = "/Library/Developer/DeviceKit/Chrome"

/// The frame's longest side, in pixels, whatever the view asks.
private let largestSide = 4096.0

/// A device type's chrome as its `chrome.json` describes it.
private struct Chrome: Decodable {
  struct Images: Decodable {
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
    let devicePadding: Insets?
  }
  struct Sizing: Decodable {
    let leftWidth: Double
    let rightWidth: Double
    let topHeight: Double
    let bottomHeight: Double
  }
  struct Insets: Decodable {
    let top: Double
    let left: Double
    let bottom: Double
    let right: Double
  }
  struct Input: Decodable {
    struct Offsets: Decodable { let normal: Point }
    struct Point: Decodable {
      let x: Double
      let y: Double
    }
    let image: String?
    let anchor: String
    let align: String?
    let onTop: Bool?
    let offsets: Offsets
  }
  /// The outline of the device, whose rounded corners the stretched pieces keep inside.
  struct Paths: Decodable {
    struct Border: Decodable {
      let insets: Insets
      let cornerRadiusX: Double
      let cornerRadiusY: Double
    }
    let simpleOutsideBorder: Border?
  }
  let images: Images
  let inputs: [Input]?
  let paths: Paths?
}

/// A PDF of the chrome, its first page and its size in points.
private struct Piece {
  let page: CGPDFPage
  let size: CGSize

  init?(_ url: URL) {
    guard let document = CGPDFDocument(url as CFURL), let page = document.page(at: 1) else { return nil }
    self.page = page
    size = page.getBoxRect(.mediaBox).size
  }

  /// Draws the page stretched into `rect` of a context whose origin is its
  /// top left. The page's drawing may reach past its box, which stretching
  /// a 2-point edge slice would spread over the whole frame, so it is cut
  /// to `rect`.
  func draw(in rect: CGRect, _ context: CGContext) {
    guard size.width > 0, size.height > 0, rect.width > 0, rect.height > 0 else { return }
    context.saveGState()
    context.clip(to: rect)
    context.translateBy(x: rect.minX, y: rect.maxY)
    context.scaleBy(x: rect.width / size.width, y: -rect.height / size.height)
    context.drawPDFPage(page)
    context.restoreGState()
  }
}

/// What the device type says of its looks.
struct DeviceLooks {
  /// The chrome bundle's resources; nil when Xcode installed none.
  let chrome: URL?
  /// The framebuffer mask, the screen's shape in its pixels; nil without one.
  let mask: URL?

  init(simulator: Simulator) {
    // The headers type the device type's bundle as an NSBundle, but Xcode 26
    // and later give a SimProfileBundle, which has no `resourceURL`; both
    // answer `bundleURL`, and the profile is in the bundle's resources.
    let bundle = simulator.device.deviceType.bundle.bundleURL
    let resources: URL? = bundle.appendingPathComponent("Contents/Resources")
    let profile = resources.flatMap { NSDictionary(contentsOf: $0.appendingPathComponent("profile.plist")) }
    let identifier = profile?["chromeIdentifier"] as? String
    let name = identifier?.split(separator: ".").last.map(String.init)
    let chrome = name.map {
      URL(fileURLWithPath: chromeRoot).appendingPathComponent("\($0).devicechrome/Contents/Resources")
    }
    self.chrome = chrome.flatMap {
      FileManager.default.fileExists(atPath: $0.appendingPathComponent("chrome.json").path) ? $0 : nil
    }
    let mask = (profile?["framebufferMask"] as? String).flatMap { name in
      resources?.appendingPathComponent("\(name).pdf")
    }
    self.mask = mask.flatMap { FileManager.default.fileExists(atPath: $0.path) ? $0 : nil }
  }

  /// The radius of the screen's corners, in the screen's own pixels, as
  /// its mask draws them: where the mask's top row turns opaque.
  func cornerRadius(screenPixels: CGSize) -> Double {
    guard let mask, let piece = Piece(mask) else { return 0 }
    let width = Int(screenPixels.width)
    guard width > 0,
      let context = CGContext(
        data: nil, width: width, height: 1, bitsPerComponent: 8, bytesPerRow: width,
        space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.alphaOnly.rawValue)
    else { return 0 }
    // The context's one row is the mask's top row.
    context.translateBy(x: 0, y: 1 - screenPixels.height)
    context.saveGState()
    context.scaleBy(x: screenPixels.width / piece.size.width, y: screenPixels.height / piece.size.height)
    context.drawPDFPage(piece.page)
    context.restoreGState()
    guard let data = context.data?.assumingMemoryBound(to: UInt8.self) else { return 0 }
    for x in 0..<(width / 2) where data[x] >= 128 {
      return Double(x)
    }
    return 0
  }
}

/// The frame, drawn: a PNG and where the screen sits in it, in its pixels.
struct DrawnFrame {
  let png: Data
  let width: Int
  let height: Int
  let screen: CGRect
  /// The radius of the screen's corners, in the frame's pixels.
  let cornerRadius: Double
}

enum FrameError: Error {
  case unreadable(String)
}

/// Draws the frame of `looks`'s chrome around a screen of `screenPoints`
/// (portrait), turned by `orientation`, fitted into `area` pixels. Nil when
/// the device type has no chrome Xcode installed.
func drawFrame(
  looks: DeviceLooks, screenPoints: CGSize, orientation: CGImagePropertyOrientation, area: CGSize,
  context ciContext: CIContext
) throws -> DrawnFrame? {
  guard let resources = looks.chrome else { return nil }
  let data = try Data(contentsOf: resources.appendingPathComponent("chrome.json"))
  let chrome = try JSONDecoder().decode(Chrome.self, from: data)
  func piece(_ name: String?) -> Piece? {
    name.flatMap { Piece(resources.appendingPathComponent("\($0).pdf")) }
  }

  // The device's outline: the composite's size, else the screen with the
  // stretching pieces' widths around it.
  let sizing = chrome.images.sizing
  let composite = piece(chrome.images.composite)
  let device: CGSize
  let screenAt: CGPoint
  if let composite {
    device = composite.size
    screenAt = CGPoint(
      x: (device.width - screenPoints.width) / 2, y: (device.height - screenPoints.height) / 2)
  } else {
    device = CGSize(
      width: screenPoints.width + sizing.leftWidth + sizing.rightWidth,
      height: screenPoints.height + sizing.topHeight + sizing.bottomHeight)
    screenAt = CGPoint(x: sizing.leftWidth, y: sizing.topHeight)
  }
  // Room outside the outline for the buttons that stand out of it.
  let padding = chrome.images.devicePadding ?? Chrome.Insets(top: 0, left: 0, bottom: 0, right: 0)
  let canvas = CGSize(
    width: device.width + padding.left + padding.right, height: device.height + padding.top + padding.bottom)
  let origin = CGPoint(x: padding.left, y: padding.top)

  let turned = orientation == .left || orientation == .right
  let upright = turned ? CGSize(width: canvas.height, height: canvas.width) : canvas
  let fit = min(area.width / upright.width, area.height / upright.height)
  let scale = min(fit, largestSide / max(upright.width, upright.height))
  guard scale > 0 else { throw FrameError.unreadable("the view asked for a frame of no size") }
  let pixels = CGSize(width: (canvas.width * scale).rounded(), height: (canvas.height * scale).rounded())

  guard
    let context = CGContext(
      data: nil, width: Int(pixels.width), height: Int(pixels.height), bitsPerComponent: 8, bytesPerRow: 0,
      space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
  else { throw FrameError.unreadable("the frame's canvas could not be made") }
  // Top-left origin, in the chrome's points.
  context.translateBy(x: 0, y: pixels.height)
  context.scaleBy(x: scale, y: -scale)

  func button(_ input: Chrome.Input) {
    guard let image = piece(input.image) else { return }
    let size = image.size
    let offset = input.offsets.normal
    let trailing = input.align == "trailing"
    let rect: CGRect
    switch input.anchor {
    case "left":
      let y = trailing ? origin.y + device.height + offset.y - size.height : origin.y + offset.y
      rect = CGRect(x: origin.x + offset.x - size.width, y: y, width: size.width, height: size.height)
    case "right":
      let y = trailing ? origin.y + device.height + offset.y - size.height : origin.y + offset.y
      rect = CGRect(x: origin.x + device.width + offset.x, y: y, width: size.width, height: size.height)
    case "top":
      let x = trailing ? origin.x + device.width + offset.x - size.width : origin.x + offset.x
      rect = CGRect(x: x, y: origin.y + offset.y - size.height, width: size.width, height: size.height)
    case "bottom":
      let x = trailing ? origin.x + device.width + offset.x - size.width : origin.x + offset.x
      rect = CGRect(x: x, y: origin.y + device.height + offset.y, width: size.width, height: size.height)
    default:
      return
    }
    image.draw(in: rect, context)
  }

  let inputs = chrome.inputs ?? []
  // Buttons behind the bezel stand out of its edge.
  for input in inputs where input.onTop != true { button(input) }
  if let composite {
    composite.draw(in: CGRect(origin: origin, size: device), context)
  } else {
    let corners = [chrome.images.topLeft, chrome.images.topRight, chrome.images.bottomLeft, chrome.images.bottomRight]
      .map(piece)
    guard let topLeft = corners[0], let topRight = corners[1], let bottomLeft = corners[2],
      let bottomRight = corners[3]
    else { throw FrameError.unreadable("the device's frame lacks a corner") }
    let left = origin.x
    let top = origin.y
    let right = origin.x + device.width
    let bottom = origin.y + device.height
    // The glass behind the bezel fills the device within its rounded
    // outline, which its corners would otherwise stand out of.
    context.saveGState()
    if let border = chrome.paths?.simpleOutsideBorder {
      let outline = CGRect(
        x: left + border.insets.left, y: top + border.insets.top,
        width: device.width - border.insets.left - border.insets.right,
        height: device.height - border.insets.top - border.insets.bottom)
      context.addPath(
        CGPath(
          roundedRect: outline, cornerWidth: min(border.cornerRadiusX, outline.width / 2),
          cornerHeight: min(border.cornerRadiusY, outline.height / 2), transform: nil))
      context.clip()
    }
    piece(chrome.images.screen)?.draw(
      in: CGRect(
        x: left + topLeft.size.width / 2, y: top + topLeft.size.height / 2,
        width: device.width - topLeft.size.width, height: device.height - topLeft.size.height), context)
    context.restoreGState()
    // Each edge reaches a point under the corners drawn over it, so no
    // seam of antialiasing shows where they meet.
    let under = 1.0
    piece(chrome.images.top)?.draw(
      in: CGRect(
        x: left + topLeft.size.width - under, y: top,
        width: device.width - topLeft.size.width - topRight.size.width + 2 * under, height: topLeft.size.height),
      context)
    piece(chrome.images.bottom)?.draw(
      in: CGRect(
        x: left + bottomLeft.size.width - under, y: bottom - bottomLeft.size.height,
        width: device.width - bottomLeft.size.width - bottomRight.size.width + 2 * under,
        height: bottomLeft.size.height),
      context)
    piece(chrome.images.left)?.draw(
      in: CGRect(
        x: left, y: top + topLeft.size.height - under, width: topLeft.size.width,
        height: device.height - topLeft.size.height - bottomLeft.size.height + 2 * under), context)
    piece(chrome.images.right)?.draw(
      in: CGRect(
        x: right - topRight.size.width, y: top + topRight.size.height - under, width: topRight.size.width,
        height: device.height - topRight.size.height - bottomRight.size.height + 2 * under), context)
    topLeft.draw(in: CGRect(origin: CGPoint(x: left, y: top), size: topLeft.size), context)
    topRight.draw(
      in: CGRect(origin: CGPoint(x: right - topRight.size.width, y: top), size: topRight.size), context)
    bottomLeft.draw(
      in: CGRect(origin: CGPoint(x: left, y: bottom - bottomLeft.size.height), size: bottomLeft.size), context)
    bottomRight.draw(
      in: CGRect(
        origin: CGPoint(x: right - bottomRight.size.width, y: bottom - bottomRight.size.height),
        size: bottomRight.size), context)
  }
  for input in inputs where input.onTop == true { button(input) }

  // The screen is cut out, in the mask's shape, for the picture beneath.
  let screen = CGRect(
    x: origin.x + screenAt.x, y: origin.y + screenAt.y, width: screenPoints.width, height: screenPoints.height)
  context.setBlendMode(.destinationOut)
  if let mask = looks.mask.flatMap(Piece.init) {
    mask.draw(in: screen, context)
  } else {
    context.fill(screen)
  }
  context.setBlendMode(.normal)
  guard let portrait = context.makeImage() else { throw FrameError.unreadable("the frame was not drawn") }

  // Turned as the picture is turned upright, the screen's place with it.
  let image = CIImage(cgImage: portrait)
  let transform = image.orientationTransform(for: orientation)
  let shown = image.oriented(orientation)
  // The screen in CoreImage's bottom-left pixels, turned, then back.
  let pixelScreen = CGRect(
    x: screen.minX * scale, y: pixels.height - screen.maxY * scale, width: screen.width * scale,
    height: screen.height * scale)
  let turnedScreen = pixelScreen.applying(transform)
  let extent = shown.extent
  let placed = CGRect(
    x: turnedScreen.minX - extent.minX, y: extent.maxY - turnedScreen.maxY, width: turnedScreen.width,
    height: turnedScreen.height)
  guard let rendered = ciContext.createCGImage(shown, from: extent) else {
    throw FrameError.unreadable("the turned frame was not drawn")
  }
  let png = NSMutableData()
  guard let destination = CGImageDestinationCreateWithData(png, UTType.png.identifier as CFString, 1, nil) else {
    throw FrameError.unreadable("the frame's PNG could not be made")
  }
  CGImageDestinationAddImage(destination, rendered, nil)
  guard CGImageDestinationFinalize(destination) else { throw FrameError.unreadable("the frame's PNG was not written") }
  return DrawnFrame(
    png: png as Data, width: rendered.width, height: rendered.height,
    screen: placed.integral.intersection(CGRect(x: 0, y: 0, width: rendered.width, height: rendered.height)),
    cornerRadius: looks.cornerRadius(
      screenPixels: CGSize(width: screenPoints.width * scale, height: screenPoints.height * scale)))
}
