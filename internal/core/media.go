package core

import (
	"bytes"
	"slices"
	"strings"
)

// AttachmentFileExtensions are the image and document types implied by attachment support.
// +demi:table
var AttachmentFileExtensions = []FileExtension{
	FileExtensionPNG,
	FileExtensionJPG,
	FileExtensionJPEG,
	FileExtensionGIF,
	FileExtensionWebP,
	FileExtensionPDF,
}

// VideoFileExtensions are the types implied only by known video support.
// +demi:table
var VideoFileExtensions = []FileExtension{FileExtensionMP4, FileExtensionMov, FileExtensionWebM, FileExtensionM4V}

// A media type a model can receive, with the extension a model's catalog
// accepts it by.
type ModelMediaType struct {
	MediaType string         `json:"mediaType"`
	Kind      ModelMediaKind `json:"kind"`
	Extension FileExtension  `json:"extension"`
}

// ModelMediaTypes is the closed set of native model media.
var ModelMediaTypes = []ModelMediaType{
	{MediaType: "image/png", Kind: ModelMediaKindImage, Extension: FileExtensionPNG},
	{MediaType: "image/jpeg", Kind: ModelMediaKindImage, Extension: FileExtensionJPEG},
	{MediaType: "image/gif", Kind: ModelMediaKindImage, Extension: FileExtensionGIF},
	{MediaType: "image/webp", Kind: ModelMediaKindImage, Extension: FileExtensionWebP},
	{MediaType: "video/mp4", Kind: ModelMediaKindVideo, Extension: FileExtensionMP4},
	{MediaType: "video/x-m4v", Kind: ModelMediaKindVideo, Extension: FileExtensionM4V},
	{MediaType: "video/quicktime", Kind: ModelMediaKindVideo, Extension: FileExtensionMov},
	{MediaType: "video/webm", Kind: ModelMediaKindVideo, Extension: FileExtensionWebM},
}

// AcceptsFileExtension reports known support for an extension; JPEG aliases
// agree, and unknown support (no accepted list) accepts nothing.
func AcceptsFileExtension(accepted *[]FileExtension, extension FileExtension) bool {
	if accepted == nil {
		return false
	}
	alias := extension
	switch extension {
	case FileExtensionJPG:
		alias = FileExtensionJPEG
	case FileExtensionJPEG:
		alias = FileExtensionJPG
	}
	return slices.Contains(*accepted, extension) || slices.Contains(*accepted, alias)
}

// ModelAcceptsVideo reports known support for at least one video type.
func ModelAcceptsVideo(model Model) bool {
	for _, extension := range VideoFileExtensions {
		if AcceptsFileExtension(model.AcceptedExtensions, extension) {
			return true
		}
	}
	return false
}

// ModelMediaTypeFor looks up a medium in the closed native-media table.
func ModelMediaTypeFor(mediaType string) (ModelMediaType, bool) {
	for _, entry := range ModelMediaTypes {
		if entry.MediaType == mediaType {
			return entry, true
		}
	}
	return ModelMediaType{}, false
}

// ModelAcceptsMediaType treats unknown catalog support as no.
func ModelAcceptsMediaType(model Model, mediaType string) bool {
	entry, ok := ModelMediaTypeFor(mediaType)
	if !ok {
		return false
	}
	return AcceptsFileExtension(model.AcceptedExtensions, entry.Extension)
}

// SniffModelMediaType recognizes only known magic numbers, with at least 12 bytes.
func SniffModelMediaType(data []byte) (ModelMediaType, bool) {
	if len(data) < 12 {
		return ModelMediaType{}, false
	}
	var mediaType string
	switch {
	case bytes.HasPrefix(data, []byte("\x89PNG")):
		mediaType = "image/png"
	case bytes.HasPrefix(data, []byte("\xff\xd8\xff")):
		mediaType = "image/jpeg"
	case bytes.HasPrefix(data, []byte("GIF87a")), bytes.HasPrefix(data, []byte("GIF89a")):
		mediaType = "image/gif"
	case string(data[:4]) == "RIFF" && string(data[8:12]) == "WEBP":
		mediaType = "image/webp"
	case bytes.HasPrefix(data, []byte("\x1a\x45\xdf\xa3")):
		mediaType = "video/webm"
	case string(data[4:8]) == "ftyp":
		switch {
		case string(data[8:12]) == "qt  ":
			mediaType = "video/quicktime"
		case string(data[8:11]) == "M4V":
			mediaType = "video/x-m4v"
		default:
			mediaType = "video/mp4"
		}
	default:
		return ModelMediaType{}, false
	}
	return ModelMediaTypeFor(mediaType)
}

// PreviewMediaType recognizes a filename's extension with ASCII case folding.
func PreviewMediaType(path string) (string, bool) {
	name := path[strings.LastIndexAny(path, "/\\")+1:]
	dot := strings.LastIndexByte(name, '.')
	if dot <= 0 {
		return "", false
	}
	extension := strings.Map(func(r rune) rune {
		if r >= 'A' && r <= 'Z' {
			return r + ('a' - 'A')
		}
		return r
	}, name[dot+1:])
	for _, entry := range PreviewTypes {
		if slices.Contains(entry.Extensions, extension) {
			return entry.MediaType, true
		}
	}
	return "", false
}

// ShowsInPlace reports whether the page can render this exact media type directly.
func ShowsInPlace(mediaType string) bool {
	for _, entry := range PreviewTypes {
		if entry.InPlace && entry.MediaType == mediaType {
			return true
		}
	}
	return false
}
