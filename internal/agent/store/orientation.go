package store

import (
	"bytes"
	"encoding/binary"
	"image"
)

// imageOrientation extracts the EXIF orientation used when an image is reencoded.
// The standard codecs and x/image do not expose EXIF; only the orientation tag
// in the first TIFF IFD is read, matching image::metadata::Orientation.
func imageOrientation(data []byte, mediaType string) uint16 {
	switch mediaType {
	case "image/jpeg":
		return jpegOrientation(data)
	case "image/png":
		return pngOrientation(data)
	case "image/webp":
		// A malformed/missing EXIF chunk has no orientation, as in Rust.
		if exif, err := webpChunk(data, "EXIF"); err == nil {
			return exifOrientation(exif)
		}
	}
	return 1
}

// exifOrientation reads the same SHORT/count-one IFD entry as the Rust image codec.
func exifOrientation(data []byte) uint16 {
	if len(data) < 8 {
		return 1
	}
	var order binary.ByteOrder
	switch string(data[:4]) {
	case "II*\x00":
		order = binary.LittleEndian
	case "MM\x00*":
		order = binary.BigEndian
	default:
		return 1
	}
	at := uint64(order.Uint32(data[4:8]))
	if at+2 > uint64(len(data)) {
		return 1
	}
	count := order.Uint16(data[at : at+2])
	at += 2
	for range count {
		if at+12 > uint64(len(data)) {
			return 1
		}
		entry := data[at : at+12]
		if order.Uint16(entry) == 0x112 && order.Uint16(entry[2:]) == 3 && order.Uint32(entry[4:]) == 1 {
			value := order.Uint16(entry[8:])
			if value >= 1 && value <= 8 {
				return value
			}
			return 1
		}
		at += 12
	}
	return 1
}

// orientImage applies EXIF before fitting, including the four mirrored orientations.
func orientImage(source image.Image, orientation uint16) image.Image {
	if orientation == 1 {
		return source
	}
	bounds := source.Bounds()
	width, height := bounds.Dx(), bounds.Dy()
	outWidth, outHeight := width, height
	if orientation >= 5 {
		outWidth, outHeight = height, width
	}
	target := fittingBuffer(image.Rect(0, 0, outWidth, outHeight), source.ColorModel())
	for y := 0; y < height; y++ {
		for x := 0; x < width; x++ {
			dx, dy := x, y
			switch orientation {
			case 2:
				dx = width - 1 - x
			case 3:
				dx, dy = width-1-x, height-1-y
			case 4:
				dy = height - 1 - y
			case 5:
				dx, dy = y, x
			case 6:
				dx, dy = height-1-y, x
			case 7:
				dx, dy = height-1-y, width-1-x
			case 8:
				dx, dy = y, width-1-x
			}
			target.Set(dx, dy, source.At(bounds.Min.X+x, bounds.Min.Y+y))
		}
	}
	return target
}

func jpegOrientation(data []byte) uint16 {
	for at := 2; at+4 <= len(data); {
		if data[at] != 0xff {
			break
		}
		marker := data[at+1]
		if marker == 0xff {
			at++
			continue
		}
		if marker == 0xda || marker == 0xd9 {
			break
		}
		if marker == 1 || marker >= 0xd0 && marker <= 0xd7 {
			at += 2
			continue
		}
		size := int(binary.BigEndian.Uint16(data[at+2:]))
		if size < 2 || size > len(data)-at-2 {
			break
		}
		chunk := data[at+4 : at+2+size]
		if marker == 0xe1 && bytes.HasPrefix(chunk, []byte("Exif\x00\x00")) {
			return exifOrientation(chunk[6:])
		}
		at += size + 2
	}
	return 1
}

func pngOrientation(data []byte) uint16 {
	for at := 8; at+12 <= len(data); {
		size := uint64(binary.BigEndian.Uint32(data[at:]))
		if size > uint64(len(data)-at-12) {
			break
		}
		end := at + 8 + int(size)
		if string(data[at+4:at+8]) == "IDAT" {
			break
		}
		if string(data[at+4:at+8]) == "eXIf" {
			return exifOrientation(data[at+8 : end])
		}
		at = end + 4
	}
	return 1
}
