// A coded sample as H.264 Annex B, which demi-mobile and the browsers take:
// VideoToolbox gives length-prefixed units and keeps its parameter sets in the
// sample's format description.

import CoreMedia
import Foundation

/// The sample's H.264 as Annex B, with the parameter sets in front of a key
/// frame, and whether it is a key frame.
func annexB(_ sample: CMSampleBuffer) -> (bytes: Data, key: Bool) {
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
  return (bytes, key)
}
