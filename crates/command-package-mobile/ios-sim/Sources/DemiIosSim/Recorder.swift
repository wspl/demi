// `record`: what the device shows, as an H.264 MP4 at the upright size it
// had when the recording started; a picture of another shape, after a
// rotation, is fitted into it.

import AVFoundation
import CoreImage
import Foundation
import QuartzCore

final class Recorder: @unchecked Sendable {
  private let writer: AVAssetWriter
  private let input: AVAssetWriterInput
  private let adaptor: AVAssetWriterInputPixelBufferAdaptor
  private let url: URL
  private let size: CGSize
  private weak var session: DeviceSession?
  private let queue = DispatchQueue(label: "demi-ios-sim.recorder")
  private let started = CACurrentMediaTime()
  private var lastTime = CMTime.zero
  private var finished = false

  init(size: CGSize, url: URL, session: DeviceSession) throws {
    let width = max(2, Int(size.width) & ~1)
    let height = max(2, Int(size.height) & ~1)
    self.size = CGSize(width: width, height: height)
    self.session = session
    self.url = url
    writer = try AVAssetWriter(outputURL: url, fileType: .mp4)
    input = AVAssetWriterInput(
      mediaType: .video,
      outputSettings: [
        AVVideoCodecKey: AVVideoCodecType.h264,
        AVVideoWidthKey: width,
        AVVideoHeightKey: height,
      ])
    input.expectsMediaDataInRealTime = true
    adaptor = AVAssetWriterInputPixelBufferAdaptor(
      assetWriterInput: input,
      sourcePixelBufferAttributes: [
        kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
        kCVPixelBufferWidthKey as String: width,
        kCVPixelBufferHeightKey as String: height,
      ])
    guard writer.canAdd(input) else { throw HelperError("the recording could not start") }
    writer.add(input)
    guard writer.startWriting() else {
      throw HelperError("the recording could not start: \(writer.error.map(describe) ?? "")")
    }
    writer.startSession(atSourceTime: .zero)
  }

  /// Adds the picture as it is now.
  func drawn() {
    queue.async { [self] in append(at: CACurrentMediaTime()) }
  }

  private func append(at time: CFTimeInterval) {
    guard !finished, let session, let image = session.upright(), input.isReadyForMoreMediaData,
      let pool = adaptor.pixelBufferPool
    else { return }
    let at = CMTime(seconds: time - started, preferredTimescale: 600)
    guard at > lastTime || lastTime == .zero else { return }
    var created: CVPixelBuffer?
    guard CVPixelBufferPoolCreatePixelBuffer(nil, pool, &created) == kCVReturnSuccess, let buffer = created else {
      return
    }
    let extent = image.extent
    let scale = min(size.width / extent.width, size.height / extent.height)
    let fitted = image
      .transformed(by: CGAffineTransform(translationX: -extent.minX, y: -extent.minY))
      .transformed(by: CGAffineTransform(scaleX: scale, y: scale))
    let centred = fitted.transformed(
      by: CGAffineTransform(
        translationX: (size.width - fitted.extent.width) / 2, y: (size.height - fitted.extent.height) / 2))
    let canvas = CIImage(color: .black).cropped(to: CGRect(origin: .zero, size: size))
    session.pictures.context.render(centred.composited(over: canvas), to: buffer)
    if adaptor.append(buffer, withPresentationTime: at) { lastTime = at }
  }

  /// Ends the recording with the picture as it is now, its MP4 left at its
  /// path for demi-mobile, and answers its length in milliseconds.
  func finish() async throws -> Int {
    let duration = await withCheckedContinuation { (done: CheckedContinuation<Int, Never>) in
      queue.async { [self] in
        let now = CACurrentMediaTime()
        // The last picture lasts until the stop, however still the screen was.
        append(at: now)
        finished = true
        input.markAsFinished()
        done.resume(returning: Int((now - started) * 1000))
      }
    }
    await writer.finishWriting()
    guard writer.status == .completed else {
      try? FileManager.default.removeItem(at: url)
      throw HelperError("the recording could not be finished: \(writer.error.map(describe) ?? "")")
    }
    return duration
  }

  /// Ends a recording nobody takes, as the device detaches: its file goes.
  func discard() async {
    _ = try? await finish()
    try? FileManager.default.removeItem(at: url)
  }
}
