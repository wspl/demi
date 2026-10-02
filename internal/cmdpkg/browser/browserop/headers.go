package browserop

import (
	"bytes"
	"encoding/binary"
	"errors"
	"fmt"
	"math"
)

// Header sizes and the video tab identity field size, in bytes.
const (
	VideoHeaderBytes = 40
	VideoTabBytes    = 16
	FileHeaderBytes  = 8
	FrameHeaderBytes = 32
)

// A video frame's header, big-endian: the tab ID (ASCII, padded with zero
// bytes to 16), the stream generation (u32), the sequence number (u32),
// flags (u8, 1 = key frame), three reserved bytes, the timestamp in
// microseconds (f64), and the picture's width and height in pixels (u16
// each).
type VideoHeader struct {
	Tab        TabID
	Generation uint32
	Sequence   uint32
	Key        bool
	Timestamp  float64
	Width      uint16
	Height     uint16
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

// A file frame's header, big-endian: the upload (u32) and the file's index
// in it (u32).
type FileHeader struct {
	Upload uint32
	File   uint32
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

// An encoded frame's header, big-endian: the capture (u32), the sequence
// number (u32), flags (u8, 1 = key frame), three reserved bytes, the
// timestamp in microseconds (f64), the picture's width and height in pixels
// (u16 each), and eight reserved bytes.
type FrameHeader struct {
	Capture   uint32
	Sequence  uint32
	Key       bool
	Timestamp float64
	Width     uint16
	Height    uint16
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
