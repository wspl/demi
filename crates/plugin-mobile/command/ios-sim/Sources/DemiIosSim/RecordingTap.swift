// `record`: every picture the device draws that differs from the one before,
// coded by the Mac's hardware encoder at the screen's own pixels and handed to
// demi-mobile as H.264 with where it differs
// (`crates/plugin-mobile/command/src/ios/helper.rs`).
//
// The simulator says each time it drew, and the picture is copied at once, on
// the thread that tells it, into a pixel buffer from a pool: a copy made later
// could show a later draw. It is compared with the picture kept last, byte for
// byte, and a draw that is the same costs nothing; the others are coded as
// they come, and the pictures the encoder holds are delivered when the
// recording stops. A draw told when no buffer is free, and a picture the
// encoder drops, are not lost to the screen but to the recording, and are
// counted: demi-mobile adds the counts up and tells them. The picture after
// a dropped one says it differs from the one before the dropped one too.

import CoreImage
import CoreMedia
import Foundation
import QuartzCore
import VideoToolbox

/// Where a picture differs from the one before, in its pixels.
struct Changed {
  var x: Int
  var y: Int
  var width: Int
  var height: Int
}

extension Changed {
  /// The smallest area around both.
  func union(_ other: Changed) -> Changed {
    let left = min(x, other.x)
    let top = min(y, other.y)
    let right = max(x + width, other.x + other.width)
    let bottom = max(y + height, other.y + other.height)
    return Changed(x: left, y: top, width: right - left, height: bottom - top)
  }
}

/// What a picture being coded carries until the encoder gives it back.
private struct Pending {
  let changed: Changed
  /// The draws not taken since the picture before.
  let missed: Int
  let width: Int
  let height: Int
}

final class RecordingTap: @unchecked Sendable {
  /// Pictures that may wait for the encoder at once; the pool holds a few more
  /// for the picture kept and the one being copied.
  private static let inFlight = 6
  private static let pool = 10

  private let recording: UInt32
  /// The pictures between key frames, which demi-mobile says: every source of
  /// a recording has the same.
  private let keyInterval: Int
  private weak var session: DeviceSession?
  private let output: Output
  private let lock = NSLock()
  private var compression: VTCompressionSession?
  private var pool: CVPixelBufferPool?
  private var size = (width: 0, height: 0)
  /// The picture kept last, which the next is compared with.
  private var before: CVPixelBuffer?
  private var waiting = 0
  /// The draws not taken since the last picture that was.
  private var missed = 0
  /// The pictures the encoder dropped since the last one it gave, and the
  /// area they and the draws before them changed.
  private var dropped = 0
  private var droppedArea: Changed?
  private var pending: [Int64: Pending] = [:]
  private var stopped = false

  init(recording: UInt32, session: DeviceSession, output: Output, keyInterval: Int) {
    self.recording = recording
    self.keyInterval = keyInterval
    self.session = session
    self.output = output
  }

  /// The device drew: its picture is copied and compared now.
  func drawn() {
    let time = CACurrentMediaTime()
    guard let session, let image = session.upright() else { return }
    // H.264 needs even sides.
    let width = max(2, Int(image.extent.width.rounded()) & ~1)
    let height = max(2, Int(image.extent.height.rounded()) & ~1)
    lock.lock()
    defer { lock.unlock() }
    if stopped { return }
    if waiting >= RecordingTap.inFlight {
      missed += 1
      return
    }
    if compression == nil || size.width != width || size.height != height {
      do {
        try restart(width: width, height: height)
      } catch {
        fail("\(error)")
        return
      }
    }
    var created: CVPixelBuffer?
    let limit = [kCVPixelBufferPoolAllocationThresholdKey: RecordingTap.pool] as CFDictionary
    guard let pool,
      CVPixelBufferPoolCreatePixelBufferWithAuxAttributes(nil, pool, limit, &created) == kCVReturnSuccess,
      let buffer = created
    else {
      missed += 1
      return
    }
    tag(buffer)
    session.pictures.render(image, into: buffer)
    guard let changed = before.map({ differs(buffer, from: $0) }) ?? Changed(x: 0, y: 0, width: width, height: height)
    else { return }
    let stamp = Int64(time * 1_000_000)
    pending[stamp] = Pending(changed: changed, missed: missed, width: width, height: height)
    missed = 0
    waiting += 1
    before = buffer
    let status = VTCompressionSessionEncodeFrame(
      compression!, imageBuffer: buffer, presentationTimeStamp: CMTime(value: stamp, timescale: 1_000_000),
      duration: .invalid, frameProperties: nil, infoFlagsOut: nil
    ) { [self] status, _, sample in
      coded(status: status, sample: sample, stamp: stamp)
    }
    if status != noErr { fail("a picture did not encode (\(status))") }
  }

  /// Delivers what the encoder still holds, then lets it go.
  func stop() {
    let held = lock.withLock { () -> VTCompressionSession? in
      stopped = true
      return compression
    }
    if let held {
      VTCompressionSessionCompleteFrames(held, untilPresentationTimeStamp: .invalid)
      VTCompressionSessionInvalidate(held)
    }
    // The pictures dropped after the last one given are told on their own.
    let left = lock.withLock { () -> Int in
      compression = nil
      before = nil
      defer {
        dropped = 0
        droppedArea = nil
      }
      return dropped
    }
    if left > 0 {
      output.control(["event": "missed", "recording": recording, "missed": left])
    }
  }

  /// A new encoder at `width` × `height`, the first picture at that size a
  /// key frame. The pictures the old one holds are delivered first.
  private func restart(width: Int, height: Int) throws {
    if let old = compression {
      // The callback needs the lock, so it is let go while the encoder is
      // emptied.
      lock.unlock()
      VTCompressionSessionCompleteFrames(old, untilPresentationTimeStamp: .invalid)
      VTCompressionSessionInvalidate(old)
      lock.lock()
    }
    var created: VTCompressionSession?
    let status = VTCompressionSessionCreate(
      allocator: nil, width: Int32(width), height: Int32(height), codecType: kCMVideoCodecType_H264,
      encoderSpecification: [kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder: true] as CFDictionary,
      imageBufferAttributes: nil, compressedDataAllocator: nil, outputCallback: nil, refcon: nil,
      compressionSessionOut: &created)
    guard status == noErr, let created else {
      throw HelperError("the hardware video encoder did not start (\(status))")
    }
    let settings: [(CFString, CFTypeRef)] = [
      (kVTCompressionPropertyKey_RealTime, kCFBooleanTrue),
      (kVTCompressionPropertyKey_AllowFrameReordering, kCFBooleanFalse),
      (kVTCompressionPropertyKey_ProfileLevel, kVTProfileLevel_H264_High_AutoLevel),
      // Every picture at a quality that keeps a one-pixel move visible; none
      // is skipped for a bit rate. Measured on a simulator scrolled without
      // pause at 1206 × 2622, 20 seconds took 13 MB at 0.5, 24 MB at 0.65,
      // 60 MB at 0.8 and 186 MB at 0.9, where a recording ends at 200 MB; a
      // white pixel drawn on a gradient decodes to nearly all of its white
      // at 0.8, to 80% of it at 0.65.
      (kVTCompressionPropertyKey_Quality, NSNumber(value: 0.8)),
      (kVTCompressionPropertyKey_ExpectedFrameRate, NSNumber(value: 60)),
      (kVTCompressionPropertyKey_MaxKeyFrameInterval, NSNumber(value: keyInterval)),
    ]
    for (key, value) in settings {
      VTSessionSetProperty(created, key: key, value: value)
    }
    VTCompressionSessionPrepareToEncodeFrames(created)
    let attributes: [CFString: Any] = [
      kCVPixelBufferPixelFormatTypeKey: kCVPixelFormatType_32BGRA,
      kCVPixelBufferWidthKey: width,
      kCVPixelBufferHeightKey: height,
      kCVPixelBufferIOSurfacePropertiesKey: [:] as CFDictionary,
    ]
    pool = nil
    CVPixelBufferPoolCreate(nil, nil, attributes as CFDictionary, &pool)
    compression = created
    size = (width, height)
    before = nil
  }

  /// The video says BT.601, the matrix an export decodes it with.
  private func tag(_ buffer: CVPixelBuffer) {
    CVBufferSetAttachment(buffer, kCVImageBufferYCbCrMatrixKey, kCVImageBufferYCbCrMatrix_ITU_R_601_4, .shouldPropagate)
    CVBufferSetAttachment(buffer, kCVImageBufferColorPrimariesKey, kCVImageBufferColorPrimaries_SMPTE_C, .shouldPropagate)
    CVBufferSetAttachment(
      buffer, kCVImageBufferTransferFunctionKey, kCVImageBufferTransferFunction_ITU_R_709_2, .shouldPropagate)
  }

  /// A picture the encoder gave back, or none: it dropped the picture, which
  /// real-time encoding does when it is behind. A dropped picture is counted
  /// and the recording goes on.
  private func coded(status: OSStatus, sample: CMSampleBuffer?, stamp: Int64) {
    guard status == noErr else {
      fail("a picture did not encode (\(status))")
      return
    }
    let given = lock.withLock { () -> (Pending, Int, Changed)? in
      waiting -= 1
      guard let taken = pending.removeValue(forKey: stamp) else { return nil }
      let drops = dropped
      let area = droppedArea.map { $0.union(taken.changed) } ?? taken.changed
      if sample == nil {
        dropped += taken.missed + 1
        droppedArea = area
        return nil
      }
      dropped = 0
      droppedArea = nil
      return (taken, taken.missed + drops, area)
    }
    guard let (taken, missed, area) = given, let sample else { return }
    let (bytes, key) = annexB(sample)
    output.picture(
      recording: recording, missed: UInt32(clamping: missed), key: key, timestamp: Double(stamp),
      width: taken.width, height: taken.height, changed: area, bytes)
  }

  private func fail(_ message: String) {
    let report = lock.withLock { () -> Bool in
      defer { stopped = true }
      return !stopped
    }
    if report {
      output.control(["event": "recording", "recording": recording, "failed": message])
    }
  }
}

/// Where `now` differs from `before`, exactly; nil when they are the same
/// picture.
private func differs(_ now: CVPixelBuffer, from before: CVPixelBuffer) -> Changed? {
  CVPixelBufferLockBaseAddress(now, .readOnly)
  CVPixelBufferLockBaseAddress(before, .readOnly)
  defer {
    CVPixelBufferUnlockBaseAddress(now, .readOnly)
    CVPixelBufferUnlockBaseAddress(before, .readOnly)
  }
  let width = CVPixelBufferGetWidth(now)
  let height = CVPixelBufferGetHeight(now)
  guard let a = CVPixelBufferGetBaseAddress(now), let b = CVPixelBufferGetBaseAddress(before),
    CVPixelBufferGetWidth(before) == width, CVPixelBufferGetHeight(before) == height
  else { return Changed(x: 0, y: 0, width: width, height: height) }
  let strideA = CVPixelBufferGetBytesPerRow(now)
  let strideB = CVPixelBufferGetBytesPerRow(before)
  let row = width * 4
  func same(_ y: Int) -> Bool { memcmp(a + y * strideA, b + y * strideB, row) == 0 }
  var top = 0
  while top < height && same(top) { top += 1 }
  if top == height { return nil }
  var bottom = height - 1
  while bottom > top && same(bottom) { bottom -= 1 }
  var left = width
  var right = -1
  for y in top...bottom where !same(y) {
    let p = (a + y * strideA).assumingMemoryBound(to: UInt32.self)
    let q = (b + y * strideB).assumingMemoryBound(to: UInt32.self)
    var first = 0
    while first < width && p[first] == q[first] { first += 1 }
    var last = width - 1
    while last > first && p[last] == q[last] { last -= 1 }
    left = min(left, first)
    right = max(right, last)
  }
  return Changed(x: left, y: top, width: right - left + 1, height: bottom - top + 1)
}
