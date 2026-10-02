package store

// API checkpoint: named parameters document the interface until bodies are ported.
//revive:disable:unused-parameter

import (
	"context"

	"github.com/wspl/demi/internal/core"
)

// SnippetMaxChars is how many Unicode scalar values a text file's opening keeps.
const SnippetMaxChars = 160

// IsText reports whether the file's media type or extension identifies text.
func IsText(name, mediaType string) bool { panic("not written: a-store") }

// Snippet reads at most 4,096 bytes, normalizes line endings, drops leading
// whitespace and keeps at most SnippetMaxChars Unicode scalar values.
func Snippet(data []byte) string { panic("not written: a-store") }

// UploadMediaType returns the sniffed native media or PDF type, else sent.
func UploadMediaType(sent string, data []byte) string { panic("not written: a-store") }

// Upload is an upload written to the conversation's Host, as its message receives it.
type Upload struct {
	// Name is the name it was written under.
	Name string
	// Path is its absolute path on the conversation's Host.
	Path string
	// MediaType is the type recorded by UploadMediaType.
	MediaType string
	SHA256    core.BlobRef
	Bytes     core.B64Bytes
}

// UploadBlocks returns a native image, video or PDF followed by its attachment
// record, plus held bytes. Fitted images are stored first when reencoded;
// an unfit image remains only an attachment record to read by path.
func UploadBlocks(ctx context.Context, upload Upload, blobs BlobStore) ([]core.UserContentBlock, HeldMedia, error) {
	panic("not written: a-store")
}

// Unavailable returns the text for an upload that is gone or belongs to another sender.
func Unavailable(reference string) core.UserContentBlock { panic("not written: a-store") }
