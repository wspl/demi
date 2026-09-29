package core

import (
	"fmt"
	"strings"
	"unicode"
)

// AttachmentTag names a file for the model without including its content.
func AttachmentTag(attachment Attachment) string {
	escape := strings.NewReplacer("&", "&amp;", "\"", "&quot;", "<", "&lt;", ">", "&gt;")
	return fmt.Sprintf(`<attachment name="%s" type="%s" size="%d" path="%s"/>`, escape.Replace(attachment.Name), escape.Replace(attachment.MediaType), attachment.SizeBytes, escape.Replace(attachment.Path))
}

// Trim removes exactly JavaScript's white space and line terminators.
func Trim(text string) string { return strings.TrimFunc(text, isTrimmedSpace) }

// IsBlank uses the same definition of white space as Trim.
func IsBlank(text string) bool { return Trim(text) == "" }

// isTrimmedSpace defines the message contract's JavaScript-compatible trim.
func isTrimmedSpace(r rune) bool { return r == '\ufeff' || unicode.IsSpace(r) && r != '\u0085' }
