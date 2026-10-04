package store

import (
	"bytes"
	"context"
	"strings"

	"golang.org/x/text/encoding/unicode"

	"github.com/wspl/demi/internal/types"
)

// SnippetMaxChars is how many Unicode scalar values a text file's opening keeps.
const SnippetMaxChars = 160

// IsText reports whether the file's media type or extension identifies text.
func IsText(name, mediaType string) bool {
	if strings.HasPrefix(mediaType, "text/") {
		return true
	}
	at := strings.LastIndexByte(name, '.')
	if at < 0 {
		return false
	}
	extension := strings.Map(func(r rune) rune {
		if r >= 'A' && r <= 'Z' {
			return r + ('a' - 'A')
		}
		return r
	}, name[at+1:])
	switch extension {
	case "txt", "md", "markdown", "log", "csv", "json", "yaml", "yml", "xml", "ini", "toml":
		return true
	}
	return false
}

// Snippet reads at most 4,096 bytes, normalizes line endings, drops leading
// whitespace and keeps at most SnippetMaxChars Unicode scalar values.
func Snippet(data []byte) string {
	// UTF-8's replacement decoder cannot fail for a byte slice; it replaces
	// each malformed sequence with U+FFFD.
	decoded, _ := unicode.UTF8.NewDecoder().Bytes(data[:min(len(data), 4096)])
	text := strings.ReplaceAll(strings.ReplaceAll(string(decoded), "\r\n", "\n"), "\r", "\n")
	trimmed := strings.TrimSpace(text)
	if trimmed == "" {
		return ""
	}
	// Locate the opening with TrimSpace, retaining its trailing whitespace.
	opening := []rune(text[strings.Index(text, trimmed):])
	return string(opening[:min(len(opening), SnippetMaxChars)])
}

// UploadMediaType returns the sniffed native media or PDF type, else sent.
func UploadMediaType(sent string, data []byte) string {
	if media, ok := types.SniffModelMediaType(data); ok {
		return media.MediaType
	}
	if bytes.HasPrefix(data, []byte("%PDF-")) {
		return "application/pdf"
	}
	return sent
}

// Upload is an upload written to the conversation's Host, as its message receives it.
type Upload struct {
	// Name is the name it was written under.
	Name string
	// Path is its absolute path on the conversation's Host.
	Path string
	// MediaType is the type recorded by UploadMediaType.
	MediaType string
	// SHA256 identifies the uploaded bytes in the blob namespace.
	SHA256 types.BlobRef
	// Bytes holds the uploaded file contents.
	Bytes types.B64Bytes
}

// UploadBlocks returns a native image, video or PDF followed by its attachment
// record, plus held bytes. Fitted images are stored first when reencoded;
// an unfit image remains only an attachment record to read by path.
func UploadBlocks(ctx context.Context, upload Upload, blobs Blobs) ([]types.UserContentBlock, HeldMedia, error) {
	record := &types.UserAttachment{
		Attachment: types.Attachment{
			Name:      upload.Name,
			Path:      upload.Path,
			MediaType: upload.MediaType,
			SizeBytes: uint64(len(upload.Bytes)),
			SHA256:    upload.SHA256,
		},
	}
	if IsText(upload.Name, upload.MediaType) {
		record.Snippet = new(Snippet(upload.Bytes))
	}
	held := HeldMedia{}
	blocks := []types.UserContentBlock{}
	media, ok := types.SniffModelMediaType(upload.Bytes)
	switch {
	case ok && media.Kind == "image":
		fitted, err := Fit(ctx, upload.Bytes, media.MediaType)
		if err == nil {
			blob := upload.SHA256
			if fitted.Reencoded {
				blob, err = blobs.Put(ctx, fitted.Data)
				if err != nil {
					return nil, HeldMedia{}, err
				}
			}
			held.Hold(blob, fitted.Data)
			blocks = append(
				blocks,
				&types.UserImage{Source: &types.MediaSourceRef{Ref: blob, MediaType: fitted.MediaType}},
			)
		}
	case ok:
		held.Hold(upload.SHA256, upload.Bytes)
		blocks = append(
			blocks,
			&types.UserVideo{Source: &types.MediaSourceRef{Ref: upload.SHA256, MediaType: media.MediaType}},
		)
	default:
		mediaType, _, _ := strings.Cut(upload.MediaType, ";")
		if strings.TrimSpace(mediaType) == "application/pdf" || bytes.HasPrefix(upload.Bytes, []byte("%PDF-")) {
			held.Hold(upload.SHA256, upload.Bytes)
			blocks = append(
				blocks,
				&types.UserDocument{
					Source: &types.DocumentRef{Ref: upload.SHA256, MediaType: "application/pdf", FileName: upload.Name},
				},
			)
		}
	}
	return append(blocks, record), held, nil
}

// Unavailable returns the text for an upload that is gone or belongs to another sender.
func Unavailable(reference string) types.UserContentBlock {
	return &types.UserText{Text: "[attachment " + reference + " is not available]"}
}
