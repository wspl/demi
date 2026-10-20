// The upright picture as a PNG, scaled so its long side is at most what
// the caller asks, and rendered into the encoders' pixel buffers.

import CoreImage
import Foundation
import ImageIO
import UniformTypeIdentifiers

final class Pictures: @unchecked Sendable {
  let context = CIContext(options: [.cacheIntermediates: false])

  /// The size of `image` scaled so its long side is at most `longSide`,
  /// never enlarged, each side rounded as demi-mobile's scale rounds it, so
  /// the screenshot is the size its coordinates assume. The extent of a
  /// turned picture is not whole pixels; its sides are.
  static func size(of image: CIImage, longSide: Int) -> (width: Int, height: Int) {
    let width = image.extent.width.rounded()
    let height = image.extent.height.rounded()
    let factor = min(1, Double(longSide) / Double(max(width, height)))
    return (max(1, Int((width * factor).rounded())), max(1, Int((height * factor).rounded())))
  }

  /// `image` moved to the origin and scaled to fill `width` × `height`.
  static func filling(_ image: CIImage, width: Double, height: Double) -> CIImage {
    let extent = image.extent
    return image
      .transformed(by: CGAffineTransform(translationX: -extent.minX, y: -extent.minY))
      .transformed(by: CGAffineTransform(scaleX: width / extent.width, y: height / extent.height))
  }

  func png(_ image: CIImage, longSide: Int) throws -> (Data, Int, Int) {
    let (width, height) = Self.size(of: image, longSide: longSide)
    let fitted = Self.filling(image, width: Double(width), height: Double(height))
    let rect = CGRect(x: 0, y: 0, width: width, height: height)
    guard let picture = context.createCGImage(fitted, from: rect, format: .RGBA8, colorSpace: CGColorSpace(name: CGColorSpace.sRGB))
    else {
      throw HelperError("the screenshot could not be drawn")
    }
    let data = NSMutableData()
    guard let destination = CGImageDestinationCreateWithData(data, UTType.png.identifier as CFString, 1, nil)
    else {
      throw HelperError("the screenshot could not be encoded")
    }
    CGImageDestinationAddImage(destination, picture, nil)
    guard CGImageDestinationFinalize(destination) else {
      throw HelperError("the screenshot could not be encoded")
    }
    return (data as Data, width, height)
  }

  /// Draws `image` into `buffer`, scaled to fill it.
  func render(_ image: CIImage, into buffer: CVPixelBuffer) {
    let width = Double(CVPixelBufferGetWidth(buffer))
    let height = Double(CVPixelBufferGetHeight(buffer))
    context.render(Self.filling(image, width: width, height: height), to: buffer)
  }
}
