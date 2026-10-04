package store

import (
	"bytes"
	"context"
	"encoding/binary"
	"errors"
	"fmt"
	"image"
	"image/color"
	"image/gif"
	"image/jpeg"
	"image/png"
	"io"
	"math"

	"golang.org/x/image/draw"
	"golang.org/x/image/webp"

	"github.com/wspl/demi/internal/types"
)

// Dimensions gives an image's width and height in pixels.
type Dimensions struct {
	// Width is the image width in pixels.
	Width uint32
	// Height is the image height in pixels.
	Height uint32
}

// Fitted is an image as it enters a transcript.
type Fitted struct {
	// Data holds the bytes entering the transcript.
	Data types.B64Bytes
	// MediaType identifies the encoding of Data.
	MediaType string
	// Came is the size in pixels as the image came.
	Came Dimensions
	// Entered is the size in pixels as the image entered.
	Entered Dimensions
	// Reencoded reports whether the image entered as different bytes.
	Reencoded bool
}

// Fit fits an image once to 2,000 pixels per side and 3,750,000 bytes.
// PNG, JPEG and WebP within both limits keep their original bytes after decoding.
// GIF uses its first frame. Reencoding applies orientation and scales as needed;
// JPEG keeps quality 90, other formats become PNG, then quality-85 JPEG if needed.
// The caller's goroutine performs codec work outside state locks.
func Fit(ctx context.Context, data types.B64Bytes, mediaType string) (fitted Fitted, err error) {
	// A decoder panic means the image is undecodable, not that the process failed.
	defer func() {
		if failed := recover(); failed != nil {
			fitted = Fitted{}
			err = fmt.Errorf("it could not be decoded (%v)", failed)
		}
	}()
	if err := ctx.Err(); err != nil {
		return Fitted{}, err
	}
	dimensions, decoded, err := decodeImage(data, mediaType)
	if err != nil {
		return Fitted{}, err
	}
	came := Dimensions{Width: uint32(dimensions.Width), Height: uint32(dimensions.Height)}
	if mediaType != "image/gif" && max(came.Width, came.Height) <= maxImageSide && len(data) <= maxImageBytes {
		return Fitted{Data: bytes.Clone(data), MediaType: mediaType, Came: came, Entered: came}, nil
	}
	if mediaType == "image/gif" {
		// A GIF's first frame may occupy only part of its logical screen.
		canvas := image.NewNRGBA(image.Rect(0, 0, dimensions.Width, dimensions.Height))
		draw.Draw(canvas, decoded.Bounds(), decoded, decoded.Bounds().Min, draw.Src)
		decoded = canvas
	}
	decoded = orientImage(decoded, imageOrientation(data, mediaType))
	decoded, scaled := scaleImage(decoded)
	entered := Dimensions{Width: uint32(decoded.Bounds().Dx()), Height: uint32(decoded.Bounds().Dy())}
	if err := ctx.Err(); err != nil {
		return Fitted{}, err
	}
	return encodeFittedImage(decoded, mediaType, came, entered, scaled)
}

const (
	maxImageSide   = 2000
	maxImageBytes  = 3750000
	maxDecodeBytes = 256 * 1024 * 1024
)

// decodeSize is the decoded buffer size at the image's own channel count, checked
// against the 256 MiB budget before decoding pixels. Go's PNG color model uses
// four channels even for RGB and gray-alpha, so PNG counts from the IHDR color type.
func decodeSize(config image.Config, mediaType string, data []byte) uint64 {
	channels := uint64(4)
	switch mediaType {
	case "image/jpeg":
		channels = 3
		if config.ColorModel == color.GrayModel {
			channels = 1
		}
	case "image/webp":
		channels = 3
		if config.ColorModel == color.NYCbCrAModel {
			channels = 4
		}
		if len(data) >= 25 && string(data[12:16]) == "VP8L" && data[24]&0x10 != 0 {
			channels = 4
		}
		if len(data) >= 21 && string(data[12:16]) == "VP8X" && data[20]&0x10 != 0 {
			channels = 4
		}
	case "image/png":
		channels = pngDecodeChannels(data)
	}
	// Division avoids overflow for malicious headers near the codec's maximum.
	row := uint64(config.Width) * channels
	if row > maxDecodeBytes || uint64(config.Height) > maxDecodeBytes/max(row, 1) {
		return maxDecodeBytes + 1
	}
	return row * uint64(config.Height)
}

// fittingBuffer retains the original sample precision during image transforms.
func fittingBuffer(bounds image.Rectangle, model color.Model) draw.Image {
	switch model {
	case color.Gray16Model:
		return image.NewGray16(bounds)
	case color.GrayModel:
		return image.NewGray(bounds)
	case color.RGBA64Model, color.NRGBA64Model:
		return image.NewNRGBA64(bounds)
	default:
		return image.NewNRGBA(bounds)
	}
}

// encodeImage writes a fitted image, discarding alpha rather than compositing for JPEG.
func encodeImage(source image.Image, mediaType string, quality int) ([]byte, error) {
	var out bytes.Buffer
	if mediaType == "image/png" {
		err := png.Encode(&out, source)
		return out.Bytes(), err
	}
	bounds := source.Bounds()
	opaque := image.NewNRGBA(image.Rect(0, 0, bounds.Dx(), bounds.Dy()))
	for y := 0; y < bounds.Dy(); y++ {
		for x := 0; x < bounds.Dx(); x++ {
			// NRGBA preserves unassociated RGB when the alpha channel is removed.
			pixel := color.NRGBAModel.Convert(source.At(bounds.Min.X+x, bounds.Min.Y+y)).(color.NRGBA)
			pixel.A = 255
			opaque.SetNRGBA(x, y, pixel)
		}
	}
	err := jpeg.Encode(&out, opaque, &jpeg.Options{Quality: quality})
	return out.Bytes(), err
}

func decodeImage(data types.B64Bytes, mediaType string) (image.Config, image.Image, error) {
	var config func(io.Reader) (image.Config, error)
	var decode func(io.Reader) (image.Image, error)
	switch mediaType {
	case "image/png":
		config, decode = png.DecodeConfig, png.Decode
	case "image/jpeg":
		config, decode = jpeg.DecodeConfig, jpeg.Decode
	case "image/gif":
		config, decode = gif.DecodeConfig, gif.Decode
	case "image/webp":
		config, decode = webp.DecodeConfig, webp.Decode
	default:
		return image.Config{}, nil, fmt.Errorf("it could not be decoded (%s is not an image type)", mediaType)
	}
	dimensions, err := config(bytes.NewReader(data))
	if err != nil {
		return image.Config{}, nil, fmt.Errorf("it could not be decoded (%w)", err)
	}
	if decodeSize(dimensions, mediaType, data) > maxDecodeBytes {
		return image.Config{}, nil, errors.New("decoding it would take more than 256 MiB")
	}
	var decoded image.Image
	if mediaType == "image/webp" && len(data) >= 30 && string(data[12:16]) == "VP8X" && data[20]&2 != 0 {
		decoded, err = firstWebPFrame(data, dimensions)
	} else {
		decoded, err = decode(bytes.NewReader(data))
	}
	if err != nil {
		return image.Config{}, nil, fmt.Errorf("it could not be decoded (%w)", err)
	}
	return dimensions, decoded, nil
}

func scaleImage(decoded image.Image) (image.Image, bool) {
	bounds := decoded.Bounds()
	scaled := max(bounds.Dx(), bounds.Dy()) > maxImageSide
	if scaled {
		ratio := float64(maxImageSide) / float64(max(bounds.Dx(), bounds.Dy()))
		width := max(1, int(math.Round(float64(bounds.Dx())*ratio)))
		height := max(1, int(math.Round(float64(bounds.Dy())*ratio)))
		// Scale to the fitted dimensions, keeping the source's sample precision.
		target := fittingBuffer(image.Rect(0, 0, width, height), decoded.ColorModel())
		draw.CatmullRom.Scale(target, target.Bounds(), decoded, bounds, draw.Src, nil)
		decoded = target
	}
	return decoded, scaled
}

func encodeFittedImage(
	decoded image.Image, mediaType string, came, entered Dimensions, scaled bool,
) (Fitted, error) {
	if scaled || mediaType == "image/gif" {
		format := "image/png"
		if mediaType == "image/jpeg" {
			format = mediaType
		}
		encoded, err := encodeImage(decoded, format, 90)
		if err != nil {
			return Fitted{}, fmt.Errorf("it could not be decoded (%w)", err)
		}
		if len(encoded) <= maxImageBytes {
			return Fitted{Data: encoded, MediaType: format, Came: came, Entered: entered, Reencoded: true}, nil
		}
	}
	encoded, err := encodeImage(decoded, "image/jpeg", 85)
	if err != nil {
		return Fitted{}, fmt.Errorf("it could not be decoded (%w)", err)
	}
	if len(encoded) > maxImageBytes {
		return Fitted{}, errors.New("even as a JPEG of quality 85 it is over 3,750,000 bytes")
	}
	return Fitted{Data: encoded, MediaType: "image/jpeg", Came: came, Entered: entered, Reencoded: true}, nil
}

// pngDecodeChannels counts bytes per PNG pixel using IHDR and pre-IDAT transparency.
func pngDecodeChannels(data []byte) uint64 {
	if len(data) < 26 {
		return 4
	}
	channels := uint64(4)
	switch data[25] {
	case 0:
		channels = 1
	case 2, 3:
		channels = 3
	case 4:
		channels = 2
	case 6:
		channels = 4
	}
	for at := 8; at+12 <= len(data); {
		size := uint64(binary.BigEndian.Uint32(data[at:]))
		if size > uint64(len(data)-at-12) {
			break
		}
		kind := string(data[at+4 : at+8])
		if kind == "IDAT" {
			break
		}
		if kind == "tRNS" {
			if data[25] == 0 {
				channels = 2
			} else {
				channels = 4
			}
		}
		at += 12 + int(size)
	}
	if data[24] == 16 {
		channels *= 2
	}
	return channels
}
