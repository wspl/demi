// demi-mobile's framing (`ios/helper.rs`): a four-byte big-endian length,
// a kind and the payload. Control frames are JSON, video frames a capture's
// pictures, blob frames a reply's file, picture frames a recording's coded
// pictures.

import Foundation

enum FrameKind: UInt8 {
  case control = 1
  case video = 2
  case blob = 3
  case picture = 4
}

/// Writes frames to standard output, one whole frame at a time.
final class Output: @unchecked Sendable {
  private let lock = NSLock()
  private let handle = FileHandle.standardOutput

  func write(_ kind: FrameKind, _ payload: Data) {
    var frame = Data(capacity: 5 + payload.count)
    frame.appendBigEndian(UInt32(1 + payload.count))
    frame.append(kind.rawValue)
    frame.append(payload)
    lock.withLock {
      // demi-mobile gone is the helper's end, which reading notices.
      try? handle.write(contentsOf: frame)
    }
  }

  func control(_ object: [String: Any]) {
    guard let data = try? JSONSerialization.data(withJSONObject: object) else {
      log("a message did not serialize")
      return
    }
    write(.control, data)
  }

  func blob(id: UInt32, _ bytes: Data) {
    var payload = Data(capacity: 4 + bytes.count)
    payload.appendBigEndian(id)
    payload.append(bytes)
    write(.blob, payload)
  }

  /// A capture's picture: its header, then the H.264 Annex B bytes.
  func video(
    capture: UInt32, sequence: UInt32, key: Bool, timestamp: Double, width: Int, height: Int,
    _ bytes: Data
  ) {
    var payload = Data(capacity: 24 + bytes.count)
    payload.appendBigEndian(capture)
    payload.appendBigEndian(sequence)
    payload.append(key ? 1 : 0)
    payload.append(contentsOf: [0, 0, 0])
    payload.appendBigEndian(timestamp.bitPattern)
    payload.appendBigEndian(UInt16(clamping: width))
    payload.appendBigEndian(UInt16(clamping: height))
    payload.append(bytes)
    write(.video, payload)
  }
}

extension Output {
  /// A recording's picture: its header, then the H.264 Annex B bytes. The
  /// header is big-endian: the recording (u32), how many draws the helper
  /// missed before this picture (u32), flags (u8, 1 = key frame) and three
  /// reserved bytes, the timestamp in microseconds (f64), the width and the
  /// height (u16 each), and where it differs from the picture before, x, y,
  /// width and height (u16 each).
  func picture(
    recording: UInt32, missed: UInt32, key: Bool, timestamp: Double, width: Int, height: Int, changed: Changed,
    _ bytes: Data
  ) {
    var payload = Data(capacity: 32 + bytes.count)
    payload.appendBigEndian(recording)
    payload.appendBigEndian(missed)
    payload.append(key ? 1 : 0)
    payload.append(contentsOf: [0, 0, 0])
    payload.appendBigEndian(timestamp.bitPattern)
    payload.appendBigEndian(UInt16(clamping: width))
    payload.appendBigEndian(UInt16(clamping: height))
    payload.appendBigEndian(UInt16(clamping: changed.x))
    payload.appendBigEndian(UInt16(clamping: changed.y))
    payload.appendBigEndian(UInt16(clamping: changed.width))
    payload.appendBigEndian(UInt16(clamping: changed.height))
    payload.append(bytes)
    write(.picture, payload)
  }
}

extension Data {
  mutating func appendBigEndian<T: FixedWidthInteger>(_ value: T) {
    Swift.withUnsafeBytes(of: value.bigEndian) { append(contentsOf: $0) }
  }
}

/// Reads frames from standard input until it ends.
struct Input {
  private let handle = FileHandle.standardInput
  private var buffer = Data()

  /// The next frame; nil once demi-mobile closed its end.
  mutating func next() -> (FrameKind, Data)? {
    while true {
      if buffer.count >= 4 {
        let length = buffer.prefix(4).reduce(0) { ($0 << 8) | Int($1) }
        if buffer.count >= 4 + length, length > 0 {
          let start = buffer.startIndex
          let kind = FrameKind(rawValue: buffer[start + 4])
          let payload = buffer.subdata(in: (start + 5)..<(start + 4 + length))
          buffer.removeSubrange(start..<(start + 4 + length))
          if let kind { return (kind, payload) }
          log("a frame of unknown kind came in")
          continue
        }
      }
      let read = handle.availableData
      if read.isEmpty { return nil }
      buffer.append(read)
    }
  }
}
