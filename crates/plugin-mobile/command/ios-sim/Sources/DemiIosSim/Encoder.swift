// One capture of the live view: the upright picture of the lit display, or of
// a foldable's display the capture names, scaled to the size the view asked
// for, encoded with VideoToolbox as H.264 High, Annex B, in real time
// without B-frames (`crates/plugin-mobile/command/src/ios/mod.rs`).
//
// A still screen sends no frame. A key frame asked for is sent at once, of the
// picture as it is. A picture drawn while the viewers' windows have no room
// waits, replaced by any newer one, until they do.

import CoreImage
import CoreMedia
import Foundation
import QuartzCore
import VideoToolbox

final class Encoder: @unchecked Sendable {
  private let capture: UInt32
  /// The foldable's display it shows; none for the lit display.
  let display: FoldDisplay?
  private let width: Int
  private let height: Int
  private weak var session: DeviceSession?
  private let output: Output
  private let queue = DispatchQueue(label: "demi-ios-sim.encoder")
  private let lock = NSLock()
  private var compression: VTCompressionSession?
  private var pool: CVPixelBufferPool?
  private var interval: Double
  private var last: CFTimeInterval = 0
  private var pending = false
  private var forceKey = true
  /// The screen drew since the last picture was encoded.
  private var changed = true
  /// The module asked for the picture once more.
  private var repeatAsked = false
  private var sequence: UInt32 = 0
  /// Frames handed to the encoder, which numbers them in this order.
  private var submitted: UInt32 = 0
  /// The newest frame the viewers had, how many more their windows take, and
  /// the order of the message that said so, since messages can overtake one
  /// another.
  private var floor: UInt32 = 0
  private var window: UInt32 = 1
  private var acked: UInt64 = 0
  /// A picture was drawn while the windows had no room.
  private var waiting = false
  private var stopped = false

  init(
    capture: UInt32, display: FoldDisplay?, width: Int, height: Int, fps: Int, bitrate: Int, session: DeviceSession,
    output: Output
  ) throws {
    // H.264 needs even sides.
    self.width = max(2, width & ~1)
    self.height = max(2, height & ~1)
    self.capture = capture
    self.display = display
    self.session = session
    self.output = output
    self.interval = 1 / Double(max(1, fps))
    // The low-latency rate control, as Chrome's real-time encoder uses it,
    // spends a still picture's repeats and a scroll's frames better at the
    // same bit rate; an encoder without it opens without it.
    var created: VTCompressionSession?
    var status = noErr
    for lowLatency in [true, false] {
      var specification: [CFString: Any] = [kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder: true]
      if lowLatency { specification[kVTVideoEncoderSpecification_EnableLowLatencyRateControl] = true }
      status = VTCompressionSessionCreate(
        allocator: nil, width: Int32(self.width), height: Int32(self.height), codecType: kCMVideoCodecType_H264,
        encoderSpecification: specification as CFDictionary,
        imageBufferAttributes: nil, compressedDataAllocator: nil, outputCallback: nil, refcon: nil,
        compressionSessionOut: &created)
      if status == noErr { break }
    }
    guard status == noErr, let created else {
      throw HelperError("the video encoder did not start (\(status))")
    }
    compression = created
    let settings: [(CFString, CFTypeRef)] = [
      (kVTCompressionPropertyKey_RealTime, kCFBooleanTrue),
      (kVTCompressionPropertyKey_AllowFrameReordering, kCFBooleanFalse),
      (kVTCompressionPropertyKey_ProfileLevel, kVTProfileLevel_H264_High_AutoLevel),
      (kVTCompressionPropertyKey_AverageBitRate, NSNumber(value: bitrate)),
      (kVTCompressionPropertyKey_ExpectedFrameRate, NSNumber(value: fps)),
      // Key frames come when a viewer asks for one.
      (kVTCompressionPropertyKey_MaxKeyFrameInterval, NSNumber(value: Int32.max)),
    ]
    for (key, value) in settings {
      VTSessionSetProperty(created, key: key, value: value)
    }
    VTCompressionSessionPrepareToEncodeFrames(created)
    let attributes: [CFString: Any] = [
      kCVPixelBufferPixelFormatTypeKey: kCVPixelFormatType_32BGRA,
      kCVPixelBufferWidthKey: self.width,
      kCVPixelBufferHeightKey: self.height,
      kCVPixelBufferIOSurfacePropertiesKey: [:] as CFDictionary,
    ]
    CVPixelBufferPoolCreate(nil, nil, attributes as CFDictionary, &pool)
  }

  /// A new picture was drawn: encoded now, or once the frame interval
  /// passed, so the last picture of a change is always sent.
  func drawn() {
    lock.withLock { changed = true }
    schedule()
  }

  /// Encodes the screen now, or once the frame interval passed and the
  /// windows have room.
  private func schedule() {
    let wait = lock.withLock { () -> Double? in
      if stopped || pending { return nil }
      if submitted &- min(floor, submitted) >= window {
        waiting = true
        return nil
      }
      let elapsed = CACurrentMediaTime() - last
      if elapsed >= interval { return 0 }
      pending = true
      return interval - elapsed
    }
    guard let wait else { return }
    queue.asyncAfter(deadline: .now() + wait) { [self] in
      lock.withLock { pending = false }
      encode()
    }
  }

  /// The viewers' windows have room for `window` frames past `sequence`, as
  /// message `order` says; a picture that waited goes now.
  func ack(sequence: UInt32, window: UInt32, order: UInt64) {
    let resume = lock.withLock { () -> Bool in
      guard order > acked else { return false }
      acked = order
      floor = sequence
      self.window = window
      let resume = waiting && submitted &- min(floor, submitted) < window
      if resume { waiting = false }
      return resume
    }
    if resume { schedule() }
  }

  /// Codes the screen as it is once more, as a delta frame marked as a
  /// repeat, as a new picture is: at once, or once the frame interval passed
  /// and the windows have room. A screen that drew meanwhile is coded
  /// unmarked.
  func repeatPicture() {
    lock.withLock { repeatAsked = true }
    schedule()
  }

  func keyFrame() {
    lock.withLock { forceKey = true }
    queue.async { [self] in encode() }
  }

  func encoding(bitrate: Int, fps: Int) {
    queue.async { [self] in
      guard let compression = lock.withLock({ self.compression }) else { return }
      VTSessionSetProperty(compression, key: kVTCompressionPropertyKey_AverageBitRate, value: NSNumber(value: bitrate))
      VTSessionSetProperty(compression, key: kVTCompressionPropertyKey_ExpectedFrameRate, value: NSNumber(value: fps))
      lock.withLock { interval = 1 / Double(max(1, fps)) }
    }
  }

  func stop() {
    queue.async { [self] in
      let compression = lock.withLock { () -> VTCompressionSession? in
        stopped = true
        defer { self.compression = nil }
        return self.compression
      }
      if let compression {
        VTCompressionSessionCompleteFrames(compression, untilPresentationTimeStamp: .invalid)
        VTCompressionSessionInvalidate(compression)
      }
    }
  }

  /// Encodes the picture as it is now, on the encoder's queue.
  private func encode() {
    guard let session, let image = session.asHeld(display), let pool,
      // A picture turned the other way than the capture is laid out waits
      // for the capture of its orientation, which demi-mobile starts.
      (image.extent.width > image.extent.height) == (width > height),
      let (compression, key, repeated) = lock.withLock({ () -> (VTCompressionSession, Bool, Bool)? in
        guard !stopped, let compression else { return nil }
        let key = forceKey
        let repeated = repeatAsked && !changed && !key
        forceKey = false
        changed = false
        repeatAsked = false
        last = CACurrentMediaTime()
        submitted &+= 1
        return (compression, key, repeated)
      })
    else { return }
    var created: CVPixelBuffer?
    guard CVPixelBufferPoolCreatePixelBuffer(nil, pool, &created) == kCVReturnSuccess, let buffer = created else {
      fail("no pixel buffer")
      return
    }
    session.pictures.render(image, into: buffer)
    let now = CACurrentMediaTime()
    let properties = key ? [kVTEncodeFrameOptionKey_ForceKeyFrame: kCFBooleanTrue] as CFDictionary : nil
    let status = VTCompressionSessionEncodeFrame(
      compression, imageBuffer: buffer, presentationTimeStamp: CMTime(seconds: now, preferredTimescale: 1_000_000),
      duration: .invalid, frameProperties: properties, infoFlagsOut: nil
    ) { [self] status, flags, sample in
      // The low-latency rate control may drop a picture for its budget: it is
      // offered again at the next interval, so the last picture of a change
      // still goes, a key frame still a key frame, a repeat still a repeat,
      // and it takes no number.
      if status == noErr, sample == nil || flags.contains(.frameDropped) {
        lock.withLock {
          submitted &-= 1
          if key { forceKey = true }
          if repeated { repeatAsked = true } else { changed = true }
        }
        schedule()
        return
      }
      guard status == noErr, let sample else {
        fail("a frame did not encode (\(status))")
        return
      }
      send(sample, timestamp: now, repeated: repeated)
    }
    if status != noErr { fail("a frame did not encode (\(status))") }
  }

  private func send(_ sample: CMSampleBuffer, timestamp: CFTimeInterval, repeated: Bool) {
    let (bytes, key) = annexB(sample)
    let sequence = lock.withLock { () -> UInt32 in
      self.sequence &+= 1
      return self.sequence
    }
    output.video(
      capture: capture, sequence: sequence, key: key, repeated: repeated && !key, timestamp: timestamp * 1_000_000,
      width: width, height: height, bytes)
  }

  private func fail(_ message: String) {
    let report = lock.withLock { () -> Bool in
      defer { stopped = true }
      return !stopped
    }
    if report {
      output.control(["event": "capture", "capture": capture, "failed": message])
    }
  }
}
