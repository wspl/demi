package core_test

import (
	"encoding/json"
	"os"
	"testing"

	"github.com/wspl/demi/internal/core"
)

// Lookup tables and magic-number cases use local fixtures and finish within one second.
func TestFileLookups(t *testing.T) {
	data, err := os.ReadFile("testdata/file-types.json")
	if err != nil {
		t.Fatal(err)
	}
	var cases map[string][][2]json.RawMessage
	if err := json.Unmarshal(data, &cases); err != nil {
		t.Fatal(err)
	}
	for _, scenario := range cases["previewMediaType"] {
		var path string
		var want *string
		if err := json.Unmarshal(scenario[0], &path); err != nil {
			t.Fatal(err)
		}
		if err := json.Unmarshal(scenario[1], &want); err != nil {
			t.Fatal(err)
		}
		got, ok := core.PreviewMediaType(path)
		if ok != (want != nil) || ok && got != *want {
			t.Errorf("preview %s: %q %v, want %v", path, got, ok, want)
		}
	}
	for _, scenario := range cases["showsInPlace"] {
		var media string
		var want bool
		if err := json.Unmarshal(scenario[0], &media); err != nil {
			t.Fatal(err)
		}
		if err := json.Unmarshal(scenario[1], &want); err != nil {
			t.Fatal(err)
		}
		if got := core.ShowsInPlace(media); got != want {
			t.Errorf("in place %s: %v", media, got)
		}
	}
	for _, entry := range core.ModelMediaTypes {
		if !core.ShowsInPlace(entry.MediaType) {
			t.Errorf("native media cannot be previewed: %s", entry.MediaType)
		}
	}
}

func TestMediaSniffing(t *testing.T) {
	for _, scenario := range []struct{ magic, want string }{
		{"\x89PNG\r\n\x1a\n", "image/png"},
		{"\xff\xd8\xff\xe0", "image/jpeg"},
		{"GIF89a", "image/gif"},
		{"GIF87a", "image/gif"},
		{"RIFF\x01\x02\x03\x04WEBP", "image/webp"},
		{"\x1a\x45\xdf\xa3", "video/webm"},
		{"\x00\x00\x00\x20ftypisom", "video/mp4"},
		{"\x00\x00\x00\x20ftypqt  ", "video/quicktime"},
		{"\x00\x00\x00\x20ftypM4V ", "video/x-m4v"},
		{"%PDF-1.7", ""},
		{"plain text here", ""},
	} {
		data := make([]byte, max(16, len(scenario.magic)))
		copy(data, scenario.magic)
		got, ok := core.SniffModelMediaType(data)
		if scenario.want == "" {
			if ok {
				t.Errorf("recognized %q: %v", scenario.magic, got)
			}
		} else if !ok || got.MediaType != scenario.want {
			t.Errorf("%q: %v, want %s", scenario.magic, got, scenario.want)
		}
	}
	png, ok := core.SniffModelMediaType(append([]byte("\x89PNG"), make([]byte, 12)...))
	if !ok || png.Kind != core.ModelMediaKindImage || png.Extension != core.FileExtensionPNG {
		t.Fatalf("PNG facts: %+v", png)
	}
	if _, ok := core.SniffModelMediaType([]byte("\x89PNG\r\n\x1a\n\x00\x00\x00")); ok {
		t.Fatal("short header recognized")
	}
}

func TestModelMediaSupport(t *testing.T) {
	for _, scenario := range []struct {
		accepted  *[]core.FileExtension
		extension core.FileExtension
		want      bool
	}{
		{nil, "png", false},
		{new([]core.FileExtension{}), "png", false},
		{new([]core.FileExtension{"png"}), "png", true},
		{new([]core.FileExtension{"png"}), "pdf", false},
		{new([]core.FileExtension{"jpeg"}), "jpg", true},
		{new([]core.FileExtension{"jpg"}), "jpeg", true},
	} {
		if got := core.AcceptsFileExtension(scenario.accepted, scenario.extension); got != scenario.want {
			t.Errorf("support %v %s: %v", scenario.accepted, scenario.extension, got)
		}
	}
	for _, scenario := range []struct {
		accepted      *[]core.FileExtension
		media         string
		native, video bool
	}{
		{new([]core.FileExtension{"png", "jpg", "jpeg", "gif", "webp"}), "image/png", true, false},
		{new([]core.FileExtension{"png", "jpg", "jpeg", "gif", "webp"}), "video/mp4", false, false},
		{new([]core.FileExtension{"png", "jpg", "jpeg", "gif", "webp"}), "application/pdf", false, false},
		{new([]core.FileExtension{"png", "mp4", "mov", "webm", "m4v"}), "video/quicktime", true, true},
		{new([]core.FileExtension{"png", "mp4", "mov", "webm", "m4v"}), "video/mp4", true, true},
		{new([]core.FileExtension{"jpg"}), "image/jpeg", true, false},
		{new([]core.FileExtension{}), "image/png", false, false},
		{nil, "image/png", false, false},
	} {
		model := core.Model{AcceptedExtensions: scenario.accepted}
		if core.ModelAcceptsMediaType(model, scenario.media) != scenario.native ||
			core.ModelAcceptsVideo(model) != scenario.video {
			t.Errorf("wrong native support: %+v", scenario)
		}
	}
}

func TestAttachmentTag(t *testing.T) {
	got := core.AttachmentTag(
		core.Attachment{
			Name:      `notes "v2" <&>.md`,
			Path:      `/home/demi/.demi/attachments/c1/notes "v2" <&>.md`,
			MediaType: "text/markdown",
			SizeBytes: 82,
			SHA256:    core.BlobRefOf([]byte("test")),
			Snippet:   new("never rendered"),
		},
	)
	want := `<attachment name="notes &quot;v2&quot; &lt;&amp;&gt;.md" type="text/markdown" size="82" ` +
		`path="/home/demi/.demi/attachments/c1/notes &quot;v2&quot; &lt;&amp;&gt;.md"/>`
	if got != want {
		t.Fatalf("%s", got)
	}
}

func TestJavaScriptWhitespace(t *testing.T) {
	for _, scenario := range []struct{ input, trimmed string }{
		{" \t ", ""},
		{"", ""},
		{" \t\r\n", ""},
		{"\ufeff\u3000\u00a0", ""},
		{"\u0085", "\u0085"},
		{" a ", "a"},
		{"\ufeff  New name \t\r\n\u3000", "New name"},
		{"\u0085name\u0085", "\u0085name\u0085"},
	} {
		if core.Trim(scenario.input) != scenario.trimmed || core.IsBlank(scenario.input) != (scenario.trimmed == "") {
			t.Errorf("wrong whitespace handling: %q", scenario.input)
		}
	}
	for _, scenario := range []struct{ count, want int }{{0, 0}, {1, 1}, {2, 5}, {3, 7}, {10, 7}} {
		if got := core.CharOffset("a😀é", scenario.count); got != scenario.want {
			t.Errorf("offset %d: %d", scenario.count, got)
		}
	}
}
