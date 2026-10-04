package types_test

import (
	"encoding/json"
	"os"
	"testing"

	"github.com/wspl/demi/internal/types"
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
		got, ok := types.PreviewMediaType(path)
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
		if got := types.ShowsInPlace(media); got != want {
			t.Errorf("in place %s: %v", media, got)
		}
	}
	for _, entry := range types.ModelMediaTypes {
		if !types.ShowsInPlace(entry.MediaType) {
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
		got, ok := types.SniffModelMediaType(data)
		if scenario.want == "" {
			if ok {
				t.Errorf("recognized %q: %v", scenario.magic, got)
			}
		} else if !ok || got.MediaType != scenario.want {
			t.Errorf("%q: %v, want %s", scenario.magic, got, scenario.want)
		}
	}
	png, ok := types.SniffModelMediaType(append([]byte("\x89PNG"), make([]byte, 12)...))
	if !ok || png.Kind != types.ModelMediaKindImage || png.Extension != types.FileExtensionPNG {
		t.Fatalf("PNG facts: %+v", png)
	}
	if _, ok := types.SniffModelMediaType([]byte("\x89PNG\r\n\x1a\n\x00\x00\x00")); ok {
		t.Fatal("short header recognized")
	}
}

func TestModelMediaSupport(t *testing.T) {
	for _, scenario := range []struct {
		accepted  *[]types.FileExtension
		extension types.FileExtension
		want      bool
	}{
		{nil, "png", false},
		{new([]types.FileExtension{}), "png", false},
		{new([]types.FileExtension{"png"}), "png", true},
		{new([]types.FileExtension{"png"}), "pdf", false},
		{new([]types.FileExtension{"jpeg"}), "jpg", true},
		{new([]types.FileExtension{"jpg"}), "jpeg", true},
	} {
		if got := types.AcceptsFileExtension(scenario.accepted, scenario.extension); got != scenario.want {
			t.Errorf("support %v %s: %v", scenario.accepted, scenario.extension, got)
		}
	}
	for _, scenario := range []struct {
		accepted      *[]types.FileExtension
		media         string
		native, video bool
	}{
		{new([]types.FileExtension{"png", "jpg", "jpeg", "gif", "webp"}), "image/png", true, false},
		{new([]types.FileExtension{"png", "jpg", "jpeg", "gif", "webp"}), "video/mp4", false, false},
		{new([]types.FileExtension{"png", "jpg", "jpeg", "gif", "webp"}), "application/pdf", false, false},
		{new([]types.FileExtension{"png", "mp4", "mov", "webm", "m4v"}), "video/quicktime", true, true},
		{new([]types.FileExtension{"png", "mp4", "mov", "webm", "m4v"}), "video/mp4", true, true},
		{new([]types.FileExtension{"jpg"}), "image/jpeg", true, false},
		{new([]types.FileExtension{}), "image/png", false, false},
		{nil, "image/png", false, false},
	} {
		model := types.Model{AcceptedExtensions: scenario.accepted}
		if types.ModelAcceptsMediaType(model, scenario.media) != scenario.native ||
			types.ModelAcceptsVideo(model) != scenario.video {
			t.Errorf("wrong native support: %+v", scenario)
		}
	}
}

func TestAttachmentTag(t *testing.T) {
	got := types.AttachmentTag(
		types.Attachment{
			Name:      `notes "v2" <&>.md`,
			Path:      `/home/demi/.demi/attachments/c1/notes "v2" <&>.md`,
			MediaType: "text/markdown",
			SizeBytes: 82,
			SHA256:    types.BlobRefOf([]byte("test")),
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
		if types.Trim(scenario.input) != scenario.trimmed || types.IsBlank(scenario.input) != (scenario.trimmed == "") {
			t.Errorf("wrong whitespace handling: %q", scenario.input)
		}
	}
	for _, scenario := range []struct{ count, want int }{{0, 0}, {1, 1}, {2, 5}, {3, 7}, {10, 7}} {
		if got := types.CharOffset("a😀é", scenario.count); got != scenario.want {
			t.Errorf("offset %d: %d", scenario.count, got)
		}
	}
}
