// `frame`: the device's frame as DeviceKit draws it
// (`crates/command-package-mobile/src/live.rs`).
//
// DemiChrome draws it once for the size the view asks for, turned as the
// picture is, with the screen cut out of it. The device type's profile names
// its chrome, and the screen's rounded corners are its framebuffer mask.

import CoreGraphics
import CoreSimulator
import DemiChrome
import FBSimulatorControl
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// Where Xcode installs the device frames.
private let chromeRoot = "/Library/Developer/DeviceKit/Chrome"

/// The frame's longest side, in pixels, whatever the view asks.
private let largestSide = 4096.0

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

  /// The screen's top left corner in an upright screen of `pixels`, as its
  /// mask draws it, shown turned `quarterTurns`; empty, a square corner,
  /// without a mask. The mask's four corners are alike, and a quarter turn
  /// lays the corner's upright edge level, which swaps its two distances
  /// and so the order of its points.
  func corner(pixels: CGSize, screenPoints: CGSize, quarterTurns: Int) -> [CGPoint] {
    let upright = mask.flatMap(ScreenMask.init)?.corner(pixels: pixels, screenPoints: screenPoints) ?? []
    return quarterTurns % 2 == 0 ? upright : upright.reversed().map { CGPoint(x: $0.y, y: $0.x) }
  }
}

/// The frame, drawn: a PNG, where the screen sits in it, in its pixels, and
/// the device's silhouette, which bounds the picture under it.
struct DrawnFrame {
  let png: Data
  /// A grey PNG of the frame's size, white where the picture may lie.
  let silhouette: Data
  let width: Int
  let height: Int
  let screen: CGRect
}

/// Draws the frame of `looks`'s chrome around a screen of `screenPoints`
/// (portrait), turned `quarterTurns` clockwise, fitted into `area` pixels
/// of a view with `backing` pixels per point: laid out in points, as
/// DeviceKit lays it out, and drawn at that scale. Nil when the device type
/// has no chrome Xcode installed.
func drawFrame(
  looks: DeviceLooks, screenPoints: CGSize, quarterTurns: Int, area: CGSize, backing: Double
) throws -> DrawnFrame? {
  guard let resources = looks.chrome else { return nil }
  guard backing > 0 else { throw ChromeError.unreadable("the view asked for a frame at no scale") }
  let data = try Data(contentsOf: resources.appendingPathComponent("chrome.json"))
  let chrome = try JSONDecoder().decode(Chrome.self, from: data)
  let artwork = ChromeArtwork(resources: resources)
  let room = CGSize(width: area.width / backing, height: area.height / backing)
  let fit = ChromeLayout.fit(chrome: chrome, screenPoints: screenPoints, area: room, quarterTurns: quarterTurns)
  var layout = try ChromeLayout(
    chrome: chrome, artworkSize: artwork.size, screenPoints: screenPoints, scale: fit, quarterTurns: quarterTurns)
  let longest = max(layout.size.width, layout.size.height) * backing
  if longest > largestSide {
    layout = try ChromeLayout(
      chrome: chrome, artworkSize: artwork.size, screenPoints: screenPoints, scale: fit * largestSide / longest,
      quarterTurns: quarterTurns)
  }

  // DeviceKit's layers: the buttons behind the bezel, the bezel, the
  // screen in it, the buttons on top. The silhouette is read before the
  // buttons on top, which lie outside the device.
  let context = try layout.canvas(backing: backing)
  layout.drawButtons(artwork, onTop: false, in: context)
  layout.drawFrame(artwork, in: context)
  layout.cutScreen(mask: looks.mask.flatMap(ScreenMask.init), screenPoints: screenPoints, in: context)
  let shape = try silhouetteImage(context)
  layout.drawButtons(artwork, onTop: true, in: context)
  guard let image = context.makeImage() else { throw ChromeError.unreadable("the frame was not drawn") }

  // The screen in the PNG's pixels.
  let points = layout.screen.applying(layout.transform)
  let screen = CGRect(
    x: points.minX * backing, y: points.minY * backing, width: points.width * backing,
    height: points.height * backing)
  return DrawnFrame(
    png: try encoded(image), silhouette: try encoded(shape), width: image.width, height: image.height,
    screen: screen)
}

/// The silhouette of what `context`, an RGBA canvas, holds, as a grey image.
private func silhouetteImage(_ context: CGContext) throws -> CGImage {
  let (width, height, row) = (context.width, context.height, context.bytesPerRow)
  guard let pixels = context.data?.assumingMemoryBound(to: UInt8.self) else {
    throw ChromeError.unreadable("the frame's canvas has no pixels")
  }
  // The canvas is premultiplied RGBA: alpha is each pixel's fourth byte.
  let alpha = (0..<(width * height)).map { pixels[($0 / width) * row + ($0 % width) * 4 + 3] }
  var shape = silhouette(alpha: alpha, width: width, height: height)
  let image = shape.withUnsafeMutableBytes { bytes in
    CGContext(
      data: bytes.baseAddress, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width,
      space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.none.rawValue)?.makeImage()
  }
  guard let image else { throw ChromeError.unreadable("the device's silhouette was not drawn") }
  return image
}

/// `image` as a PNG.
private func encoded(_ image: CGImage) throws -> Data {
  let png = NSMutableData()
  guard let destination = CGImageDestinationCreateWithData(png, UTType.png.identifier as CFString, 1, nil) else {
    throw ChromeError.unreadable("the frame's PNG could not be made")
  }
  CGImageDestinationAddImage(destination, image, nil)
  guard CGImageDestinationFinalize(destination) else { throw ChromeError.unreadable("the frame's PNG was not written") }
  return png as Data
}
