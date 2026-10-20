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

  /// The radius of the screen's corners in a screen of `pixels`, as its
  /// mask draws them; 0 without a mask.
  func cornerRadius(pixels: CGSize, screenPoints: CGSize) -> Double {
    mask.flatMap(ScreenMask.init)?.cornerRadius(pixels: pixels, screenPoints: screenPoints) ?? 0
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
  // screen in it, the buttons on top.
  let context = try layout.canvas(backing: backing)
  layout.drawButtons(artwork, onTop: false, in: context)
  layout.drawFrame(artwork, in: context)
  layout.cutScreen(mask: looks.mask.flatMap(ScreenMask.init), screenPoints: screenPoints, in: context)
  layout.drawButtons(artwork, onTop: true, in: context)
  guard let image = context.makeImage() else { throw ChromeError.unreadable("the frame was not drawn") }

  let png = NSMutableData()
  guard let destination = CGImageDestinationCreateWithData(png, UTType.png.identifier as CFString, 1, nil) else {
    throw ChromeError.unreadable("the frame's PNG could not be made")
  }
  CGImageDestinationAddImage(destination, image, nil)
  guard CGImageDestinationFinalize(destination) else { throw ChromeError.unreadable("the frame's PNG was not written") }
  // The screen and its corners in the PNG's pixels.
  let points = layout.screen.applying(layout.transform)
  let screen = CGRect(
    x: points.minX * backing, y: points.minY * backing, width: points.width * backing,
    height: points.height * backing)
  let upright = CGSize(width: layout.screen.width * backing, height: layout.screen.height * backing)
  return DrawnFrame(
    png: png as Data, width: image.width, height: image.height, screen: screen,
    cornerRadius: looks.cornerRadius(pixels: upright, screenPoints: screenPoints))
}
