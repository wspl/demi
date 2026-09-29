package core

import (
	"slices"
	"strings"
)

// One media type the product knows files by. Its schema types the table
// the page receives in `@demicodes/protocol`.
//
//demi:wire open
type PreviewType struct {
	MediaType string `json:"mediaType"`
	// Lowercase, without the dot.
	Extensions []string `json:"extensions"`
	// Served as itself for the page to show in place; otherwise the page
	// downloads it or renders it from its text.
	InPlace bool `json:"inPlace"`
}

var PreviewTypes = []PreviewType{
	{MediaType: "image/png", Extensions: []string{"png"}, InPlace: true},
	{MediaType: "image/jpeg", Extensions: []string{"jpg", "jpeg"}, InPlace: true},
	{MediaType: "image/gif", Extensions: []string{"gif"}, InPlace: true},
	{MediaType: "image/webp", Extensions: []string{"webp"}, InPlace: true},
	{MediaType: "image/avif", Extensions: []string{"avif"}, InPlace: true},
	{MediaType: "image/bmp", Extensions: []string{"bmp"}, InPlace: true},
	{MediaType: "image/x-icon", Extensions: []string{"ico"}, InPlace: true},
	{MediaType: "image/svg+xml", Extensions: []string{"svg"}, InPlace: true},
	{MediaType: "video/mp4", Extensions: []string{"mp4"}, InPlace: true},
	{MediaType: "video/x-m4v", Extensions: []string{"m4v"}, InPlace: true},
	{MediaType: "video/webm", Extensions: []string{"webm"}, InPlace: true},
	{MediaType: "video/quicktime", Extensions: []string{"mov"}, InPlace: true},
	{MediaType: "audio/mpeg", Extensions: []string{"mp3"}, InPlace: true},
	{MediaType: "audio/wav", Extensions: []string{"wav"}, InPlace: true},
	{MediaType: "audio/ogg", Extensions: []string{"ogg", "oga", "opus"}, InPlace: true},
	{MediaType: "audio/mp4", Extensions: []string{"m4a"}, InPlace: true},
	{MediaType: "audio/aac", Extensions: []string{"aac"}, InPlace: true},
	{MediaType: "audio/flac", Extensions: []string{"flac"}, InPlace: true},
	{MediaType: "audio/webm", Extensions: []string{"weba"}, InPlace: true},
	{MediaType: "application/pdf", Extensions: []string{"pdf"}, InPlace: true},
	{MediaType: "text/markdown", Extensions: []string{"md", "markdown"}, InPlace: false},
}

func PreviewMediaType(path string) (string, bool) {
	name := path[strings.LastIndexAny(path, `/\`)+1:]
	dot := strings.LastIndexByte(name, '.')
	if dot <= 0 {
		return "", false
	}
	extension := name[dot+1:]
	// ASCII case folding matches the Rust lookup; Unicode lookalikes do not.
	extension = strings.Map(func(r rune) rune {
		if r >= 'A' && r <= 'Z' {
			return r + ('a' - 'A')
		}
		return r
	}, extension)
	for _, entry := range PreviewTypes {
		if slices.Contains(entry.Extensions, extension) {
			return entry.MediaType, true
		}
	}
	return "", false
}
func ShowsInPlace(mediaType string) bool {
	for _, entry := range PreviewTypes {
		if entry.InPlace && entry.MediaType == mediaType {
			return true
		}
	}
	return false
}
