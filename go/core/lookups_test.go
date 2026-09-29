package core_test

import (
	"encoding/json/v2"
	"testing"

	"github.com/wspl/demi/go/core"
)

func TestFilePreviews(t *testing.T) {
	var cases map[string][][]any
	if err := json.Unmarshal(fixture(t, "file-types"), &cases); err != nil {
		t.Fatal(err)
	}
	for _, test := range cases["previewMediaType"] {
		got, ok := core.PreviewMediaType(test[0].(string))
		if test[1] == nil {
			if ok {
				t.Errorf("recognized %s as %s", test[0], got)
			}
		} else if !ok || got != test[1] {
			t.Errorf("%s: %s != %s", test[0], got, test[1])
		}
	}
	for _, test := range cases["showsInPlace"] {
		if core.ShowsInPlace(test[0].(string)) != test[1].(bool) {
			t.Error(test)
		}
	}
}

func TestModelMedia(t *testing.T) {
	for _, test := range []struct{ prefix, mime string }{
		{"\x89PNG\r\n\x1a\n", "image/png"}, {"\xff\xd8\xff\xe0", "image/jpeg"},
		{"GIF87a", "image/gif"}, {"GIF89a", "image/gif"}, {"RIFF\x01\x02\x03\x04WEBP", "image/webp"},
		{"\x1a\x45\xdf\xa3", "video/webm"}, {"\x00\x00\x00\x20ftypisom", "video/mp4"},
		{"\x00\x00\x00\x20ftypqt  ", "video/quicktime"}, {"\x00\x00\x00\x20ftypM4V ", "video/x-m4v"},
		{"%PDF-1.7", ""}, {"plain text here", ""}, {"RIFFxxxxWAVE", ""},
	} {
		data := make([]byte, 16)
		copy(data, test.prefix)
		got := core.SniffModelMediaType(data)
		if test.mime == "" {
			if got != nil {
				t.Error("recognized", test.prefix)
			}
			continue
		}
		if got == nil || got.MediaType != test.mime {
			t.Errorf("%q: %v", test.prefix, got)
			continue
		}
		if !core.ShowsInPlace(got.MediaType) {
			t.Error("browser cannot show model media", got)
		}
		accepted := []core.FileExtension{got.Extension}
		if !core.ModelAcceptsMediaType(core.Model{AcceptedExtensions: &accepted}, test.mime) {
			t.Error("catalog support ignored")
		}
	}
	if core.SniffModelMediaType([]byte("\x89PNG\r\n\x1a\n\x00\x00\x00")) != nil {
		t.Fatal("recognized short header")
	}
	if core.ModelAcceptsMediaType(core.Model{}, "image/png") || core.ModelAcceptsVideo(core.Model{}) {
		t.Fatal("unknown support accepted")
	}
	for _, test := range []struct {
		extensions *[]core.FileExtension
		extension  core.FileExtension
		want       *bool
	}{
		{nil, core.FileExtensionPng, nil},
		{new([]core.FileExtension{}), core.FileExtensionPng, new(false)},
		{new([]core.FileExtension{core.FileExtensionPng}), core.FileExtensionPng, new(true)},
		{new([]core.FileExtension{core.FileExtensionPng}), core.FileExtensionPdf, new(false)},
		{new([]core.FileExtension{core.FileExtensionJpg}), core.FileExtensionJpeg, new(true)},
		{new([]core.FileExtension{core.FileExtensionJpeg}), core.FileExtensionJpg, new(true)},
	} {
		got := core.FileExtensionSupport(test.extensions, test.extension)
		if (got == nil) != (test.want == nil) || got != nil && *got != *test.want {
			t.Errorf("extension support: %v", test)
		}
	}
	if !core.ModelAcceptsVideo(core.Model{AcceptedExtensions: new([]core.FileExtension{core.FileExtensionMp4})}) {
		t.Fatal("video not recognized")
	}
}

func TestModelFacingText(t *testing.T) {
	attachment := core.Attachment{Name: `notes "v2" <&>.md`, Path: `/notes "v2" <&>.md`, MediaType: "text/markdown", SizeBytes: 82, Snippet: new("never rendered")}
	want := `<attachment name="notes &quot;v2&quot; &lt;&amp;&gt;.md" type="text/markdown" size="82" path="/notes &quot;v2&quot; &lt;&amp;&gt;.md"/>`
	if got := core.AttachmentTag(attachment); got != want {
		t.Fatalf("attachment: %s", got)
	}
	for _, test := range []struct{ input, trimmed string }{
		{"", ""}, {" \t\r\n", ""}, {"\ufeff\u3000\u00a0", ""}, {"\u0085", "\u0085"},
		{" a ", "a"}, {"\ufeff  New name \t\r\n\u3000", "New name"}, {"\u0085name\u0085", "\u0085name\u0085"},
	} {
		if core.Trim(test.input) != test.trimmed || core.IsBlank(test.input) != (test.trimmed == "") {
			t.Errorf("trim %q", test.input)
		}
	}
}
