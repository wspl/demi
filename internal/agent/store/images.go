package store

// API checkpoint: named parameters document the interface until bodies are ported.
//revive:disable:unused-parameter

import (
	"context"

	"github.com/wspl/demi/internal/core"
)

// Dimensions gives an image's width and height in pixels.
type Dimensions struct{ Width, Height uint32 }

// Fitted is an image as it enters a transcript.
type Fitted struct {
	Data      core.B64Bytes
	MediaType string
	// Came is the size in pixels as the image came.
	Came Dimensions
	// Entered is the size in pixels as the image entered.
	Entered Dimensions
	// Reencoded reports whether the image entered as different bytes.
	Reencoded bool
}

// UnfitKind identifies why an image cannot enter a transcript.
type UnfitKind uint8

const (
	// Undecodable means the image could not be decoded.
	Undecodable UnfitKind = iota
	// TooLargeToDecode means decoding would exceed 256 MiB.
	TooLargeToDecode
	// TooLarge means quality-85 JPEG still exceeds 3,750,000 bytes.
	TooLarge
)

// Unfit describes a fitting failure, retaining the decoder's underlying error.
type Unfit struct {
	Kind  UnfitKind
	Cause error
}

// Error renders the Rust image refusal text.
func (e *Unfit) Error() string { panic("not written: a-store") }

// Unwrap returns the decoder error, when present.
func (e *Unfit) Unwrap() error { panic("not written: a-store") }

// Fit fits an image once to 2,000 pixels per side and 3,750,000 bytes.
// PNG, JPEG and WebP within both limits keep their original bytes after decoding.
// GIF uses its first frame. Reencoding applies orientation and scales as needed;
// JPEG keeps quality 90, other formats become PNG, then quality-85 JPEG if needed.
// The caller's goroutine performs codec work outside state locks.
func Fit(ctx context.Context, data core.B64Bytes, mediaType string) (Fitted, error) {
	panic("not written: a-store")
}
