package core

import "bytes"

//demi:enum
type ModelMediaKind string

const (
	ModelMediaKindImage ModelMediaKind = "image"
	ModelMediaKindVideo ModelMediaKind = "video"
)

func (k ModelMediaKind) Name() string { return string(k) }

type ModelMediaType struct {
	MediaType string
	Kind      ModelMediaKind
	Extension FileExtension
}

var ModelMediaTypes = []ModelMediaType{
	{"image/png", ModelMediaKindImage, FileExtensionPng},
	{"image/jpeg", ModelMediaKindImage, FileExtensionJpeg},
	{"image/gif", ModelMediaKindImage, FileExtensionGif},
	{"image/webp", ModelMediaKindImage, FileExtensionWebp},
	{"video/mp4", ModelMediaKindVideo, FileExtensionMp4},
	{"video/x-m4v", ModelMediaKindVideo, FileExtensionM4v},
	{"video/quicktime", ModelMediaKindVideo, FileExtensionMov},
	{"video/webm", ModelMediaKindVideo, FileExtensionWebm},
}

func ModelMediaTypeFor(mediaType string) *ModelMediaType {
	for _, entry := range ModelMediaTypes {
		if entry.MediaType == mediaType {
			return &entry
		}
	}
	return nil
}
func ModelAcceptsMediaType(model Model, mediaType string) bool {
	entry := ModelMediaTypeFor(mediaType)
	if entry == nil {
		return false
	}
	supported := FileExtensionSupport(model.AcceptedExtensions, entry.Extension)
	return supported != nil && *supported
}
func SniffModelMediaType(data []byte) *ModelMediaType {
	if len(data) < 12 {
		return nil
	}
	var mediaType string
	switch {
	case bytes.HasPrefix(data, []byte("\x89PNG")):
		mediaType = "image/png"
	case bytes.HasPrefix(data, []byte("\xff\xd8\xff")):
		mediaType = "image/jpeg"
	case bytes.HasPrefix(data, []byte("GIF87a")), bytes.HasPrefix(data, []byte("GIF89a")):
		mediaType = "image/gif"
	case bytes.HasPrefix(data, []byte("RIFF")) && string(data[8:12]) == "WEBP":
		mediaType = "image/webp"
	case bytes.HasPrefix(data, []byte("\x1a\x45\xdf\xa3")):
		mediaType = "video/webm"
	case string(data[4:8]) == "ftyp":
		switch {
		case string(data[8:12]) == "qt  ":
			mediaType = "video/quicktime"
		case bytes.HasPrefix(data[8:12], []byte("M4V")):
			mediaType = "video/x-m4v"
		default:
			mediaType = "video/mp4"
		}
	default:
		return nil
	}
	return ModelMediaTypeFor(mediaType)
}
