package store

import (
	"bytes"
	"encoding/binary"
	"fmt"
	"image"
	"image/draw"
	"io"

	"golang.org/x/image/riff"
	"golang.org/x/image/webp"
)

// webpChunk reads one container chunk with x/image's checked RIFF reader.
func webpChunk(data []byte, wanted string) ([]byte, error) {
	form, chunks, err := riff.NewReader(bytes.NewReader(data))
	if err != nil {
		return nil, err
	}
	if string(form[:]) != "WEBP" {
		return nil, fmt.Errorf("webp: invalid format")
	}
	for {
		tag, _, chunk, err := chunks.Next()
		if err != nil {
			return nil, err
		}
		if string(tag[:]) == wanted {
			return io.ReadAll(chunk)
		}
	}
}

// firstWebPFrame decodes an animation's first frame on its full canvas.
// x/image/webp decodes static bitstreams but does not expose ANMF frames. Supply
// its existing codec the first frame's chunks in a static RIFF envelope; the
// declared canvas and frame bounds are checked before it allocates pixels.
func firstWebPFrame(data []byte, config image.Config) (image.Image, error) {
	frame, err := webpChunk(data, "ANMF")
	if err != nil {
		return nil, err
	}
	if len(frame) < 16 {
		return nil, fmt.Errorf("webp: invalid ANMF chunk")
	}
	// ANMF has four little-endian 24-bit geometry fields.
	x := int(frame[0]) | int(frame[1])<<8 | int(frame[2])<<16
	y := int(frame[3]) | int(frame[4])<<8 | int(frame[5])<<16
	width := (int(frame[6]) | int(frame[7])<<8 | int(frame[8])<<16) + 1
	height := (int(frame[9]) | int(frame[10])<<8 | int(frame[11])<<16) + 1
	x *= 2
	y *= 2
	if x+width > config.Width || y+height > config.Height {
		return nil, fmt.Errorf("webp: frame outside image")
	}
	// VP8X is needed for a lossy frame with a separate ALPH chunk.
	encoded := make([]byte, 30, 30+len(frame)-16)
	copy(encoded, "RIFF")
	copy(encoded[8:], "WEBPVP8X")
	binary.LittleEndian.PutUint32(encoded[16:], 10)
	encoded[20] = data[20] & 0x10
	encoded[24], encoded[25], encoded[26] = byte(width-1), byte((width-1)>>8), byte((width-1)>>16)
	encoded[27], encoded[28], encoded[29] = byte(height-1), byte((height-1)>>8), byte((height-1)>>16)
	encoded = append(encoded, frame[16:]...)
	binary.LittleEndian.PutUint32(encoded[4:], uint32(len(encoded)-8))
	frameConfig, err := webp.DecodeConfig(bytes.NewReader(encoded))
	if err != nil {
		return nil, err
	}
	if frameConfig.Width != width || frameConfig.Height != height {
		return nil, fmt.Errorf("webp: inconsistent frame dimensions")
	}
	decoded, err := webp.Decode(bytes.NewReader(encoded))
	if err != nil {
		return nil, err
	}
	canvas := image.NewNRGBA(image.Rect(0, 0, config.Width, config.Height))
	draw.Draw(canvas, image.Rect(x, y, x+width, y+height), decoded, decoded.Bounds().Min, draw.Src)
	return canvas, nil
}
