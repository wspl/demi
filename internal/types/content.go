package types

import (
	"crypto/sha256"
	"encoding/base64"
	"fmt"
	"strings"
	"unicode"
)

// BlobRefOf names the blob by its SHA-256 digest.
func BlobRefOf(data []byte) BlobRef {
	return BlobRef(fmt.Sprintf("%x", sha256.Sum256(data)))
}

// Base64Len is the number of bytes in the padded JSON string's content.
func (b B64Bytes) Base64Len() uint64 {
	return uint64(base64.StdEncoding.EncodedLen(len(b)))
}

// String describes bytes without exposing potentially large media.
func (b B64Bytes) String() string {
	return fmt.Sprintf("B64Bytes(%d bytes)", len(b))
}

// AttachmentTag describes an uploaded file to the model without its content.
func AttachmentTag(a Attachment) string {
	// XML's encoder escapes quotes numerically; the transcript uses these named entities.
	escape := strings.NewReplacer("&", "&amp;", "\"", "&quot;", "<", "&lt;", ">", "&gt;")
	return fmt.Sprintf(
		`<attachment name="%s" type="%s" size="%d" path="%s"/>`,
		escape.Replace(a.Name),
		escape.Replace(a.MediaType),
		a.SizeBytes,
		escape.Replace(a.Path),
	)
}

// Trim removes JavaScript's whitespace and line terminators.
func Trim(text string) string {
	return strings.TrimFunc(text, func(r rune) bool {
		return r == '\ufeff' || (r != '\u0085' && unicode.IsSpace(r))
	})
}

// IsBlank reports whether JavaScript's trim leaves no text.
func IsBlank(text string) bool {
	return Trim(text) == ""
}

// CharOffset is the byte offset after chars Unicode scalar values, capped at the end.
func CharOffset(text string, chars int) int {
	for offset := range text {
		if chars <= 0 {
			return offset
		}
		chars--
	}
	return len(text)
}
