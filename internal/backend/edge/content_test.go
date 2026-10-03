package edge

import (
	"net/http"
	"net/url"
	"reflect"
	"testing"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
)

func TestMediaInPlaceImagesInertAndRestDownloads(t *testing.T) {
	for _, test := range []struct {
		media             string
		download          bool
		name, disposition string
	}{
		{"text/html", false, "", "attachment"},
		{"", false, "", "attachment"},
		{"text/markdown", false, "README.md", "attachment; filename=\"README.md\"; filename*=UTF-8''README.md"},
		{
			"image/png", true, "图 (1)'s.png",
			"attachment; filename=\"_ (1)'s.png\"; filename*=UTF-8''%E5%9B%BE%20%281%29%27s.png",
		},
	} {
		headers := contentHeaders(test.media, test.download, test.name)
		want := http.Header{
			"Content-Type":           {"application/octet-stream"},
			"Content-Disposition":    {test.disposition},
			"X-Content-Type-Options": {"nosniff"},
		}
		if !reflect.DeepEqual(headers, want) {
			t.Fatal(headers)
		}
	}
	for _, media := range []string{"image/svg+xml", "audio/mpeg", "application/pdf"} {
		name := ""
		if media == "image/svg+xml" {
			name = "logo.svg"
		}
		headers := contentHeaders(media, false, name)
		if headers.Get("Content-Type") != media {
			t.Fatal(headers)
		}
		policy := ""
		if media == "image/svg+xml" {
			policy = imagePolicy
		}
		want := http.Header{"Content-Type": {media}, "X-Content-Type-Options": {"nosniff"}}
		if policy != "" {
			want.Set("Content-Security-Policy", policy)
		}
		if !reflect.DeepEqual(headers, want) {
			t.Fatal(headers)
		}
	}
}

func TestDeleteKeepsRootsAndDirectoriesHoldingThem(t *testing.T) {
	kept := []string{"/home/ana", "/home/ana/work"}
	for _, path := range []string{"/", "/home", "/home/ana", "/HOME/Ana/", "/home/ana/work/..", "/home/ana/work", "C:\\"} {
		if !protectedPath(path, kept...) {
			t.Fatal(path)
		}
	}
	for _, path := range []string{"/home/ana/work/notes.md", "/home/ana/other", "/home/anabel"} {
		if protectedPath(path, kept...) {
			t.Fatal(path)
		}
	}
}

func TestFileVersionMetadataAndLastSegment(t *testing.T) {
	timestamp, err := core.TimestampFromMillisecond(1790000000123)
	if err != nil {
		t.Fatal(err)
	}
	stat := host.FileStat{Kind: host.File, Mode: 0o644, Size: 300000, Modified: timestamp}
	date, err := lastModified(stat)
	if err != nil || date != "Mon, 21 Sep 2026 14:13:20 GMT" {
		t.Fatal(date, err)
	}
	if fileName("/work/logo.svg") != "logo.svg" || fileName("C:\\work\\report.pdf") != "report.pdf" {
		t.Fatal("filename lost")
	}
}

func TestProductOriginIsPublicOriginOrRequestedHost(t *testing.T) {
	public, err := url.Parse("https://demi.example.com/")
	if err != nil {
		t.Fatal(err)
	}
	for _, test := range []struct {
		origin, host string
		public       *url.URL
		want         bool
	}{
		{"https://demi.example.com", "10.0.0.2:3271", public, true},
		{"http://127.0.0.1:3271", "127.0.0.1:3271", nil, true},
		{"https://demi.example.com", "demi.example.com", nil, true},
		{"https://evil.example", "demi.example.com", public, false},
		{"null", "demi.example.com", public, false},
		{"https://a1b2.expose.demi.example.com", "demi.example.com", public, false},
		{"http://127.0.0.1:9999", "127.0.0.1:3271", public, false},
		{"https://demi.example.com\xff", "demi.example.com", public, false},
		{"https://demi.example.com:444", "demi.example.com", nil, false},
	} {
		if got := productOrigin(test.origin, test.host, test.public); got != test.want {
			t.Fatalf("%+v: %v", test, got)
		}
	}
}
