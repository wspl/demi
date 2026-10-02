package storetest

// API checkpoint: named parameters document the interface until bodies are ported.
//revive:disable:unused-parameter

import (
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

// ModelOf is a model selection with a 100,000-token window.
func ModelOf(provider, model string) core.ModelSelection { panic("not written: a-store") }

// ModelReading selects a model that reads the extensions natively.
func ModelReading(provider, model string, extensions []core.FileExtension) core.ModelSelection {
	panic("not written: a-store")
}

// TestModel selects test-model from provider stub.
func TestModel() core.ModelSelection { panic("not written: a-store") }

// Text is a message's content of one text.
func Text(text string) []core.UserContentBlock { panic("not written: a-store") }

// SentText is the same content as a provider request carries it.
func SentText(text string) []provider.UserPart { panic("not written: a-store") }

// PNG produces a real PNG whose pixels follow seed.
func PNG(width, height uint32, seed uint8) core.B64Bytes { panic("not written: a-store") }
