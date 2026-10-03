package storetest

import (
	"bytes"
	"image"
	"image/color"
	"image/png"
	"slices"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

// ModelOf is a model selection with a 100,000-token window.
func ModelOf(provider, model string) core.ModelSelection {
	return core.ModelSelection{
		ProviderID: provider,
		Model: core.Model{
			ID:                 model,
			Name:               model,
			ContextWindow:      100000,
			Thinking:           []core.ThinkingCapability{},
			AcceptedExtensions: new([]core.FileExtension{}),
		},
	}
}

// ModelReading selects a model that reads the extensions natively.
func ModelReading(provider, model string, extensions []core.FileExtension) core.ModelSelection {
	selection := ModelOf(provider, model)
	selection.Model.AcceptedExtensions = new(slices.Clone(extensions))
	return selection
}

// TestModel selects test-model from provider stub.
func TestModel() core.ModelSelection { return ModelOf("stub", "test-model") }

// Text is a message's content of one text.
func Text(text string) []core.UserContentBlock {
	return []core.UserContentBlock{&core.UserText{Text: text}}
}

// SentText is the same content as a provider request carries it.
func SentText(text string) []provider.UserPart {
	return []provider.UserPart{&provider.TextPart{Text: text}}
}

// PNG produces a real PNG whose pixels follow seed.
func PNG(width, height uint32, seed uint8) core.B64Bytes {
	pixels := image.NewNRGBA(image.Rect(0, 0, int(width), int(height)))
	for y := 0; y < int(height); y++ {
		for x := 0; x < int(width); x++ {
			pixels.SetNRGBA(x, y, color.NRGBA{uint8(x), uint8(y), seed, 255})
		}
	}
	var out bytes.Buffer
	// A bytes.Buffer cannot fail a write; valid fixture dimensions always encode.
	if err := png.Encode(&out, pixels); err != nil {
		panic(err)
	}
	return out.Bytes()
}
