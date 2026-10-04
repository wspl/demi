package browserproto

import (
	"bytes"
	"encoding/binary"
	"errors"
	"fmt"
	"math"
)

// Header sizes and the video tab identity field size, in bytes.
const (
	// VideoHeaderBytes is the length of the video frame header.
	VideoHeaderBytes = 40
	// VideoTabBytes is the length of the padded video tab identity.
	VideoTabBytes = 16
	// FileHeaderBytes is the length of the file frame header.
	FileHeaderBytes = 8
	// FrameHeaderBytes is the length of the encoded capture frame header.
	FrameHeaderBytes = 32
)

// VideoHeader is a video frame's header, big-endian: the tab ID (ASCII, padded with zero
// bytes to 16), the stream generation (u32), the sequence number (u32),
// flags (u8, 1 = key frame), three reserved bytes, the timestamp in
// microseconds (f64), and the picture's width and height in pixels (u16
// each).
type VideoHeader struct {
	// Tab is the public tab identity.
	Tab TabID
	// Generation is the video stream generation.
	Generation uint32
	// Sequence is the frame sequence number.
	Sequence uint32
	// Key is whether the frame is a key frame.
	Key bool
	// Timestamp is the frame timestamp in microseconds.
	Timestamp float64
	// Width is the picture width in pixels.
	Width uint16
	// Height is the picture height in pixels.
	Height uint16
}

// Append appends the header to dst, refusing an invalid public tab identity.
func (h VideoHeader) Append(dst []byte) ([]byte, error) {
	if err := h.Tab.Validate(); err != nil {
		return nil, fmt.Errorf("tab: %w", err)
	}
	var header [VideoHeaderBytes]byte
	copy(header[:VideoTabBytes], h.Tab)
	binary.BigEndian.PutUint32(header[16:20], h.Generation)
	binary.BigEndian.PutUint32(header[20:24], h.Sequence)
	if h.Key {
		header[24] = 1
	}
	binary.BigEndian.PutUint64(header[28:36], math.Float64bits(h.Timestamp))
	binary.BigEndian.PutUint16(header[36:38], h.Width)
	binary.BigEndian.PutUint16(header[38:40], h.Height)
	return append(dst, header[:]...), nil
}

// SplitVideoFrame splits a video frame's payload into its header and data.
func SplitVideoFrame(payload []byte) (VideoHeader, []byte, error) {
	if len(payload) < VideoHeaderBytes {
		return VideoHeader{}, nil, errors.New("a video frame is shorter than its header")
	}
	tabBytes := payload[:VideoTabBytes]
	if i := bytes.IndexByte(tabBytes, 0); i >= 0 {
		tabBytes = tabBytes[:i]
	}
	tab, err := ParseTabID(string(tabBytes))
	if err != nil {
		return VideoHeader{}, nil, fmt.Errorf("tab: %w", err)
	}
	return VideoHeader{
		Tab:        tab,
		Generation: binary.BigEndian.Uint32(payload[16:20]),
		Sequence:   binary.BigEndian.Uint32(payload[20:24]),
		Key:        payload[24]&1 == 1,
		Timestamp:  math.Float64frombits(binary.BigEndian.Uint64(payload[28:36])),
		Width:      binary.BigEndian.Uint16(payload[36:38]),
		Height:     binary.BigEndian.Uint16(payload[38:40]),
	}, payload[VideoHeaderBytes:], nil
}

// FileHeader is a file frame's header, big-endian: the upload (u32) and the file's index
// in it (u32).
type FileHeader struct {
	// Upload is the upload identity.
	Upload uint32
	// File is the file index in the upload.
	File uint32
}

// SplitFileFrame splits a file frame's payload into its header and data.
func SplitFileFrame(payload []byte) (FileHeader, []byte, error) {
	if len(payload) < FileHeaderBytes {
		return FileHeader{}, nil, errors.New("a file frame is shorter than its header")
	}
	return FileHeader{
		Upload: binary.BigEndian.Uint32(payload[:4]),
		File:   binary.BigEndian.Uint32(payload[4:8]),
	}, payload[FileHeaderBytes:], nil
}

// FrameHeader is an encoded frame's header, big-endian: the capture (u32), the sequence
// number (u32), flags (u8, 1 = key frame), three reserved bytes, the
// timestamp in microseconds (f64), the picture's width and height in pixels
// (u16 each), and eight reserved bytes.
type FrameHeader struct {
	// Capture is the capture identity.
	Capture uint32
	// Sequence is the frame sequence number.
	Sequence uint32
	// Key is whether the frame is a key frame.
	Key bool
	// Timestamp is the frame timestamp in microseconds.
	Timestamp float64
	// Width is the picture width in pixels.
	Width uint16
	// Height is the picture height in pixels.
	Height uint16
}

// SplitCaptureFrame splits a binary message into its header and H.264 data.
func SplitCaptureFrame(payload []byte) (FrameHeader, []byte, error) {
	if len(payload) < FrameHeaderBytes {
		return FrameHeader{}, nil, fmt.Errorf("a capture frame of %d bytes is shorter than its header", len(payload))
	}
	return FrameHeader{
		Capture:   binary.BigEndian.Uint32(payload[:4]),
		Sequence:  binary.BigEndian.Uint32(payload[4:8]),
		Key:       payload[8]&1 == 1,
		Timestamp: math.Float64frombits(binary.BigEndian.Uint64(payload[12:20])),
		Width:     binary.BigEndian.Uint16(payload[20:22]),
		Height:    binary.BigEndian.Uint16(payload[22:24]),
	}, payload[FrameHeaderBytes:], nil
}
