// One capture of the live view: the upright picture scaled to the size the view
// asked for, encoded with VideoToolbox as H.264 High, Annex B, in real time
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

  init(capture: UInt32, width: Int, height: Int, fps: Int, bitrate: Int, session: DeviceSession, output: Output) throws {
    // H.264 needs even sides.
    self.width = max(2, width & ~1)
    self.height = max(2, height & ~1)
    self.capture = capture
    self.session = session
    self.output = output
    self.interval = 1 / Double(max(1, fps))
    var created: VTCompressionSession?
    let status = VTCompressionSessionCreate(
      allocator: nil, width: Int32(self.width), height: Int32(self.height), codecType: kCMVideoCodecType_H264,
      encoderSpecification: [kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder: true] as CFDictionary,
      imageBufferAttributes: nil, compressedDataAllocator: nil, outputCallback: nil, refcon: nil,
      compressionSessionOut: &created)
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
    if resume { drawn() }
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
    guard let session, let image = session.upright(), let pool,
      let (compression, key) = lock.withLock({ () -> (VTCompressionSession, Bool)? in
        guard !stopped, let compression else { return nil }
        let key = forceKey
        forceKey = false
        last = CACurrentMediaTime()
        submitted &+= 1
        return (compression, key)
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
    ) { [self] status, _, sample in
      guard status == noErr, let sample else {
        fail("a frame did not encode (\(status))")
        return
      }
      send(sample, timestamp: now)
    }
    if status != noErr { fail("a frame did not encode (\(status))") }
  }

  private func send(_ sample: CMSampleBuffer, timestamp: CFTimeInterval) {
    let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: false) as? [[CFString: Any]]
    let key = !(attachments?.first?[kCMSampleAttachmentKey_NotSync] as? Bool ?? false)
    var bytes = Data()
    if key, let format = CMSampleBufferGetFormatDescription(sample) {
      var count = 0
      CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
        format, parameterSetIndex: 0, parameterSetPointerOut: nil, parameterSetSizeOut: nil,
        parameterSetCountOut: &count, nalUnitHeaderLengthOut: nil)
      for index in 0..<count {
        var pointer: UnsafePointer<UInt8>?
        var size = 0
        if CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
          format, parameterSetIndex: index, parameterSetPointerOut: &pointer, parameterSetSizeOut: &size,
          parameterSetCountOut: nil, nalUnitHeaderLengthOut: nil) == noErr, let pointer
        {
          bytes.append(contentsOf: [0, 0, 0, 1])
          bytes.append(pointer, count: size)
        }
      }
    }
    if let block = CMSampleBufferGetDataBuffer(sample) {
      var length = 0
      var pointer: UnsafeMutablePointer<Int8>?
      CMBlockBufferGetDataPointer(block, atOffset: 0, lengthAtOffsetOut: nil, totalLengthOut: &length, dataPointerOut: &pointer)
      if let pointer {
        // AVCC's four-byte lengths become Annex B's start codes.
        let raw = UnsafeRawPointer(pointer).assumingMemoryBound(to: UInt8.self)
        var at = 0
        while at + 4 <= length {
          let size = (Int(raw[at]) << 24) | (Int(raw[at + 1]) << 16) | (Int(raw[at + 2]) << 8) | Int(raw[at + 3])
          at += 4
          guard size > 0, at + size <= length else { break }
          bytes.append(contentsOf: [0, 0, 0, 1])
          bytes.append(raw + at, count: size)
          at += size
        }
      }
    }
    let sequence = lock.withLock { () -> UInt32 in
      self.sequence &+= 1
      return self.sequence
    }
    output.video(
      capture: capture, sequence: sequence, key: key, timestamp: timestamp * 1_000_000, width: width, height: height,
      bytes)
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
