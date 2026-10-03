package edge

import (
	"fmt"
	"net/http"
	"path"
	"strings"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
)

const imagePolicy = "default-src 'none'; style-src 'unsafe-inline'; sandbox"

func contentHeaders(mediaType *string, download bool, name string) http.Header {
	headers := http.Header{"X-Content-Type-Options": {"nosniff"}}
	if mediaType != nil && !download && core.ShowsInPlace(*mediaType) {
		headers.Set("Content-Type", *mediaType)
		if strings.HasPrefix(*mediaType, "image/") {
			headers.Set("Content-Security-Policy", imagePolicy)
		}
	} else {
		headers.Set("Content-Type", "application/octet-stream")
		headers.Set("Content-Disposition", attachment(name))
	}
	return headers
}
func attachment(name string) string {
	if name == "" {
		return "attachment"
	}
	var fallback, encoded strings.Builder
	for _, char := range name {
		if char < ' ' || char > '~' || char == '"' || char == '\\' {
			fallback.WriteByte('_')
		} else {
			fallback.WriteRune(char)
		}
	}
	for _, char := range []byte(name) {
		if char >= 'a' && char <= 'z' || char >= 'A' && char <= 'Z' || char >= '0' && char <= '9' || strings.ContainsRune("-._~!", rune(char)) {
			encoded.WriteByte(char)
		} else {
			fmt.Fprintf(&encoded, "%%%02X", char)
		}
	}
	return fmt.Sprintf("attachment; filename=\"%s\"; filename*=UTF-8''%s", fallback.String(), encoded.String())
}
func fileName(name string) string {
	if strings.Contains(name, "\\") {
		name = strings.ReplaceAll(name, "\\", "/")
	}
	name = strings.TrimRight(name, "/")
	last := path.Base(name)
	if last == "." || last == ".." || last == "/" {
		return ""
	}
	return last
}
func protectedPath(name string, kept ...string) bool {
	normalize := func(value string) string { return path.Clean(strings.ReplaceAll(strings.ToLower(value), "\\", "/")) }
	top := normalize(name)
	if top == "/" || len(top) == 2 && top[1:] == ":" {
		return true
	}
	for _, directory := range kept {
		directory = normalize(directory)
		if directory == top || strings.HasPrefix(directory, strings.TrimRight(top, "/")+"/") {
			return true
		}
	}
	return false
}
func lastModified(stat host.FileStat) (string, error) {
	value, err := stat.Modified.Time()
	if err != nil {
		return "", err
	}
	return value.UTC().Format(http.TimeFormat), nil
}
func rawFileHeaders() http.Header {
	return http.Header{"Cache-Control": {"private, no-cache"}, "Vary": {"Cookie"}, "X-Accel-Buffering": {"no"}}
}
func addHeaders(to, from http.Header) {
	for name, values := range from {
		to[name] = values
	}
}
