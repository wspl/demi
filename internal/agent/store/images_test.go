package store_test

import (
	"bytes"
	"encoding/binary"
	"errors"
	"fmt"
	"hash/crc32"
	"image"
	"image/color"
	"image/gif"
	"image/jpeg"
	"image/png"
	"os"
	"reflect"
	"testing"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
)

func TestImageKeepsOriginalOrFitsDimensions(t *testing.T) {
	small := storetest.PNG(60, 40, 1)
	tall := image.NewNRGBA(image.Rect(0, 0, 30, 2400))
	for y := 0; y < 2400; y++ {
		for x := 0; x < 30; x++ {
			tall.SetNRGBA(x, y, color.NRGBA{uint8(x), uint8(y), 7, 255})
		}
	}
	var jpg bytes.Buffer
	if err := jpeg.Encode(&jpg, tall, &jpeg.Options{Quality: 95}); err != nil {
		t.Fatal(err)
	}
	palette := color.Palette{}
	for i := 0; i < 256; i++ {
		palette = append(palette, color.NRGBA{uint8(i), 0, 0, 255})
	}
	first := image.NewPaletted(image.Rect(0, 0, 40, 30), palette)
	for y := 0; y < 30; y++ {
		for x := 0; x < 40; x++ {
			first.SetColorIndex(x, y, uint8(x))
		}
	}
	var animated bytes.Buffer
	if err := gif.Encode(&animated, first, nil); err != nil {
		t.Fatal(err)
	}
	cases := []struct {
		name, media, wantMedia string
		data                   []byte
		came, entered          store.Dimensions
		reencoded              bool
	}{
		{
			"small",
			"image/png",
			"image/png",
			small,
			store.Dimensions{Width: 60, Height: 40},
			store.Dimensions{Width: 60, Height: 40},
			false,
		},
		{
			"wide",
			"image/png",
			"image/png",
			storetest.PNG(3000, 30, 2),
			store.Dimensions{Width: 3000, Height: 30},
			store.Dimensions{Width: 2000, Height: 20},
			true,
		},
		{
			"tall",
			"image/jpeg",
			"image/jpeg",
			jpg.Bytes(),
			store.Dimensions{Width: 30, Height: 2400},
			store.Dimensions{Width: 25, Height: 2000},
			true,
		},
		{
			"gif",
			"image/gif",
			"image/png",
			animated.Bytes(),
			store.Dimensions{Width: 40, Height: 30},
			store.Dimensions{Width: 40, Height: 30},
			true,
		},
	}
	for _, scenario := range cases {
		t.Run(scenario.name, func(t *testing.T) {
			fitted, err := store.Fit(t.Context(), scenario.data, scenario.media)
			if err != nil {
				t.Fatal(err)
			}
			if fitted.MediaType != scenario.wantMedia || fitted.Came != scenario.came ||
				fitted.Entered != scenario.entered ||
				fitted.Reencoded != scenario.reencoded {
				t.Fatalf("fit metadata: %#v", fitted)
			}
			if !scenario.reencoded && !bytes.Equal(fitted.Data, scenario.data) {
				t.Fatal("original bytes changed")
			}
			config, format, err := image.DecodeConfig(bytes.NewReader(fitted.Data))
			if err != nil || "image/"+format != scenario.wantMedia || config.Width != int(scenario.entered.Width) ||
				config.Height != int(scenario.entered.Height) {
				t.Fatalf("encoded image: %#v %s %v", config, format, err)
			}
			again, err := store.Fit(t.Context(), scenario.data, scenario.media)
			if err != nil || !reflect.DeepEqual(again, fitted) {
				t.Fatal("fitting is not deterministic")
			}
		})
	}
}

func TestLargeImageUsesJPEGAndBrokenImageIsRefused(t *testing.T) {
	// Half a megapixel of uncompressed 16-bit RGBA proves the byte limit in
	// under a second. Go PNG drops fully opaque alpha, so retain it at 65534.
	noise := image.NewNRGBA64(image.Rect(0, 0, 700, 700))
	state := uint32(1)
	next := func() uint16 {
		state = state*1664525 + 1013904223
		return uint16(state >> 16)
	}
	for y := 0; y < 700; y++ {
		for x := 0; x < 700; x++ {
			noise.SetNRGBA64(x, y, color.NRGBA64{next(), next(), next(), 65534})
		}
	}
	var encoded bytes.Buffer
	encoder := png.Encoder{CompressionLevel: png.NoCompression}
	if err := encoder.Encode(&encoded, noise); err != nil {
		t.Fatal(err)
	}
	if encoded.Len() <= 3750000 {
		t.Fatal("fixture does not reach byte limit")
	}
	fitted, err := store.Fit(t.Context(), encoded.Bytes(), "image/png")
	if err != nil {
		t.Fatal(err)
	}
	if fitted.MediaType != "image/jpeg" || !fitted.Reencoded ||
		fitted.Came != (store.Dimensions{Width: 700, Height: 700}) ||
		fitted.Entered != fitted.Came ||
		len(fitted.Data) > 3750000 {
		t.Fatalf("byte-limited fit: %s %v %v %d", fitted.MediaType, fitted.Came, fitted.Entered, len(fitted.Data))
	}
	_, err = store.Fit(t.Context(), []byte("\x89PNG\r\n\x1a\n\x00\x00\x00\x01broken"), "image/png")
	var unfit *store.Unfit
	if !errors.As(err, &unfit) || unfit.Kind != store.Undecodable {
		t.Fatalf("broken image: %v", err)
	}
}

func TestWebPOriginalIsKept(t *testing.T) {
	data, err := os.ReadFile("testdata/gopher.webp")
	if err != nil {
		t.Fatal(err)
	}
	fitted, err := store.Fit(t.Context(), data, "image/webp")
	if err != nil {
		t.Fatal(err)
	}
	if fitted.Reencoded || fitted.MediaType != "image/webp" || !bytes.Equal(fitted.Data, data) {
		t.Fatal("WebP original changed")
	}
}

// orientedPNG puts an EXIF SHORT orientation in an actual PNG before IDAT.
func orientedPNG(data []byte, orientation uint16) []byte {
	exif := []byte{'I', 'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0}
	binary.LittleEndian.PutUint16(exif[18:], orientation)
	chunk := make([]byte, 12+len(exif))
	binary.BigEndian.PutUint32(chunk, uint32(len(exif)))
	copy(chunk[4:], "eXIf")
	copy(chunk[8:], exif)
	binary.BigEndian.PutUint32(chunk[len(chunk)-4:], crc32.ChecksumIEEE(chunk[4:len(chunk)-4]))
	result := append([]byte{}, data[:33]...)
	result = append(result, chunk...)
	return append(result, data[33:]...)
}

func TestOrientationOnlyAppliedWhenReencoded(t *testing.T) {
	data := orientedPNG(storetest.PNG(6, 4, 1), 6)
	fitted, err := store.Fit(t.Context(), data, "image/png")
	if err != nil || !bytes.Equal(data, fitted.Data) || fitted.Reencoded ||
		fitted.Entered != (store.Dimensions{Width: 6, Height: 4}) {
		t.Fatalf("original orientation changed: %v", err)
	}
	for _, orientation := range []uint16{2, 3, 4, 5, 6, 7, 8} {
		t.Run(string(rune('0'+orientation)), func(t *testing.T) {
			data := orientedPNG(storetest.PNG(3000, 30, 2), orientation)
			fitted, err := store.Fit(t.Context(), data, "image/png")
			if err != nil {
				t.Fatal(err)
			}
			want := store.Dimensions{Width: 2000, Height: 20}
			if orientation >= 5 {
				want = store.Dimensions{Width: 20, Height: 2000}
			}
			if fitted.Entered != want || !fitted.Reencoded {
				t.Fatalf("orientation %d dimensions: %v", orientation, fitted.Entered)
			}
		})
	}
}

func TestImageDecodeMemoryLimit(t *testing.T) {
	data := storetest.PNG(1, 1, 1)
	binary.BigEndian.PutUint32(data[16:20], 100000)
	binary.BigEndian.PutUint32(data[20:24], 100000)
	binary.BigEndian.PutUint32(data[29:33], crc32.ChecksumIEEE(data[12:29]))
	_, err := store.Fit(t.Context(), data, "image/png")
	var unfit *store.Unfit
	if !errors.As(err, &unfit) || unfit.Kind != store.TooLargeToDecode {
		t.Fatalf("decode budget: %v", err)
	}
}

func TestAnimatedWebPUsesFirstCanvas(t *testing.T) {
	data, err := os.ReadFile("testdata/gopher.webp")
	if err != nil {
		t.Fatal(err)
	}
	// The fixture's first bitstream is 75 by 100, placed within a 2400 by 100 canvas.
	original, err := store.Fit(t.Context(), data, "image/webp")
	if err != nil {
		t.Fatal(err)
	}
	width, height := original.Came.Width, original.Came.Height
	animation := make([]byte, 30)
	copy(animation, "RIFF")
	copy(animation[8:], "WEBPVP8X")
	binary.LittleEndian.PutUint32(animation[16:], 10)
	animation[20] = 0x12
	animation[24], animation[25] = byte(2399&255), byte(2399>>8)
	animation[27] = byte(height - 1)
	animation = append(animation, []byte{'A', 'N', 'I', 'M', 6, 0, 0, 0, 0, 0, 0, 0, 0, 0}...)
	frame := make([]byte, 24)
	copy(frame, "ANMF")
	binary.LittleEndian.PutUint32(frame[4:], uint32(16+len(data)-12))
	frame[14] = byte(width - 1)
	frame[17] = byte(height - 1)
	frame = append(frame, data[12:]...)
	animation = append(animation, frame...)
	binary.LittleEndian.PutUint32(animation[4:], uint32(len(animation)-8))
	fitted, err := store.Fit(t.Context(), animation, "image/webp")
	if err != nil {
		t.Fatal(err)
	}
	if fitted.Came.Width != 2400 || fitted.Entered.Width != 2000 || fitted.MediaType != "image/png" ||
		!fitted.Reencoded {
		t.Fatalf("animated fit: %s %v %v", fitted.MediaType, fitted.Came, fitted.Entered)
	}
}

func TestMirroredOrientationsMovePixels(t *testing.T) {
	colors := []color.NRGBA{{255, 0, 0, 255}, {0, 255, 0, 255}, {0, 0, 255, 255}, {255, 255, 0, 255}}
	source := image.NewNRGBA(image.Rect(0, 0, 2400, 12))
	for y := 0; y < 12; y++ {
		for x := 0; x < 2400; x++ {
			index := 0
			if x >= 1200 {
				index++
			}
			if y >= 6 {
				index += 2
			}
			source.SetNRGBA(x, y, colors[index])
		}
	}
	var encoded bytes.Buffer
	if err := png.Encode(&encoded, source); err != nil {
		t.Fatal(err)
	}
	corners := map[uint16][]int{
		2: {1, 0, 3, 2},
		3: {3, 2, 1, 0},
		4: {2, 3, 0, 1},
		5: {0, 2, 1, 3},
		6: {2, 0, 3, 1},
		7: {3, 1, 2, 0},
		8: {1, 3, 0, 2},
	}
	for orientation, want := range corners {
		t.Run(fmt.Sprint(orientation), func(t *testing.T) {
			fitted, err := store.Fit(t.Context(), orientedPNG(encoded.Bytes(), orientation), "image/png")
			if err != nil {
				t.Fatal(err)
			}
			decoded, err := png.Decode(bytes.NewReader(fitted.Data))
			if err != nil {
				t.Fatal(err)
			}
			width, height := decoded.Bounds().Dx(), decoded.Bounds().Dy()
			positions := []image.Point{
				{X: 0, Y: 0},
				{X: width - 1, Y: 0},
				{X: 0, Y: height - 1},
				{X: width - 1, Y: height - 1},
			}
			for index, point := range positions {
				if got := color.NRGBAModel.Convert(decoded.At(point.X, point.Y)); got != colors[want[index]] {
					t.Fatalf("orientation %d corner %d: %v, want %v", orientation, index, got, colors[want[index]])
				}
			}
		})
	}
}
