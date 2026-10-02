package core_test

import (
	"encoding/json"
	"os"
	"reflect"
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
	for _, tc := range cases["previewMediaType"] {
		var path string
		var want *string
		if err := json.Unmarshal(tc[0], &path); err != nil {
			t.Fatal(err)
		}
		if err := json.Unmarshal(tc[1], &want); err != nil {
			t.Fatal(err)
		}
		if got := core.PreviewMediaType(path); !reflect.DeepEqual(got, want) {
			t.Errorf("preview %s: %v, want %v", path, got, want)
		}
	}
	for _, tc := range cases["showsInPlace"] {
		var media string
		var want bool
		if err := json.Unmarshal(tc[0], &media); err != nil {
			t.Fatal(err)
		}
		if err := json.Unmarshal(tc[1], &want); err != nil {
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
	for _, tc := range []struct{ magic, want string }{
		{"\x89PNG\r\n\x1a\n", "image/png"}, {"\xff\xd8\xff\xe0", "image/jpeg"},
		{"GIF89a", "image/gif"}, {"GIF87a", "image/gif"}, {"RIFF\x01\x02\x03\x04WEBP", "image/webp"},
		{"\x1a\x45\xdf\xa3", "video/webm"}, {"\x00\x00\x00\x20ftypisom", "video/mp4"},
		{"\x00\x00\x00\x20ftypqt  ", "video/quicktime"}, {"\x00\x00\x00\x20ftypM4V ", "video/x-m4v"},
		{"%PDF-1.7", ""}, {"plain text here", ""},
	} {
		data := make([]byte, max(16, len(tc.magic)))
		copy(data, tc.magic)
		got := core.SniffModelMediaType(data)
		if tc.want == "" {
			if got != nil {
				t.Errorf("recognized %q: %v", tc.magic, got)
			}
		} else if got == nil || got.MediaType != tc.want {
			t.Errorf("%q: %v, want %s", tc.magic, got, tc.want)
		}
	}
	if core.SniffModelMediaType([]byte("\x89PNG\r\n\x1a\n\x00\x00\x00")) != nil {
		t.Fatal("short header recognized")
	}
}

func TestModelMediaSupport(t *testing.T) {
	for _, tc := range []struct {
		accepted  *[]core.FileExtension
		extension core.FileExtension
		want      *bool
	}{
		{nil, "png", nil}, {new([]core.FileExtension{}), "png", new(false)},
		{new([]core.FileExtension{"png"}), "png", new(true)}, {new([]core.FileExtension{"png"}), "pdf", new(false)},
		{new([]core.FileExtension{"jpeg"}), "jpg", new(true)}, {new([]core.FileExtension{"jpg"}), "jpeg", new(true)},
	} {
		if got := core.FileExtensionSupport(tc.accepted, tc.extension); !reflect.DeepEqual(got, tc.want) {
			t.Errorf("support %v %s: %v", tc.accepted, tc.extension, got)
		}
	}
	for _, tc := range []struct {
		accepted      *[]core.FileExtension
		media         string
		native, video bool
	}{
		{new([]core.FileExtension{"png", "jpg", "jpeg", "gif", "webp"}), "image/png", true, false},
		{new([]core.FileExtension{"png"}), "video/mp4", false, false},
		{new([]core.FileExtension{"pdf"}), "application/pdf", false, false},
		{new([]core.FileExtension{"png", "mp4", "mov", "webm", "m4v"}), "video/quicktime", true, true},
		{new([]core.FileExtension{"mp4"}), "video/mp4", true, true},
		{new([]core.FileExtension{"jpg"}), "image/jpeg", true, false},
		{new([]core.FileExtension{}), "image/png", false, false}, {nil, "image/png", false, false},
	} {
		model := core.Model{AcceptedExtensions: tc.accepted}
		if core.ModelAcceptsMediaType(model, tc.media) != tc.native || core.ModelAcceptsVideo(model) != tc.video {
			t.Errorf("wrong native support: %+v", tc)
		}
	}
}

func TestAttachmentTag(t *testing.T) {
	got := core.AttachmentTag(core.Attachment{Name: `notes "v2" <&>.md`, Path: `/home/demi/.demi/attachments/c1/notes "v2" <&>.md`, MediaType: "text/markdown", SizeBytes: 82, Snippet: new("never rendered")})
	want := `<attachment name="notes &quot;v2&quot; &lt;&amp;&gt;.md" type="text/markdown" size="82" path="/home/demi/.demi/attachments/c1/notes &quot;v2&quot; &lt;&amp;&gt;.md"/>`
	if got != want {
		t.Fatalf("%s", got)
	}
}

func TestJavaScriptWhitespace(t *testing.T) {
	for _, tc := range []struct{ input, trimmed string }{
		{"", ""}, {" \t\r\n", ""}, {"\ufeff\u3000\u00a0", ""}, {"\u0085", "\u0085"}, {" a ", "a"},
		{"\ufeff  New name \t\r\n\u3000", "New name"}, {"\u0085name\u0085", "\u0085name\u0085"},
	} {
		if core.Trim(tc.input) != tc.trimmed || core.IsBlank(tc.input) != (tc.trimmed == "") {
			t.Errorf("wrong whitespace handling: %q", tc.input)
		}
	}
	for _, tc := range []struct{ count, want int }{{0, 0}, {1, 1}, {2, 5}, {3, 7}, {10, 7}} {
		if got := core.CharOffset("a😀é", tc.count); got != tc.want {
			t.Errorf("offset %d: %d", tc.count, got)
		}
	}
}
