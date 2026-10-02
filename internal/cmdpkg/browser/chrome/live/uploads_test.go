package live

import (
	"errors"
	"os"
	"testing"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
)

// Local temporary files only; upload validation costs less than one second.
func TestUploadKeepsChosenNamesAndSkipsEmptyFiles(t *testing.T) {
	request := &browserop.LiveViewerMessageUpload{Files: []browserop.UploadFile{{Name: "empty.txt", Size: 0}, {Name: "chosen.txt", Size: 3}}}
	u, err := prepareUpload(t.TempDir(), request)
	if err != nil {
		t.Fatal(err)
	}
	defer u.close()
	if u.current != 1 {
		t.Fatal("empty file was not completed")
	}
	if err := u.receive(1, []byte("a")); err != nil {
		t.Fatal(err)
	}
	if err := u.receive(1, []byte("bc")); err != nil {
		t.Fatal(err)
	}
	if u.current != 2 {
		t.Fatal("completed upload still expects bytes")
	}
	for i, want := range []string{"", "abc"} {
		data, err := os.ReadFile(u.files[i].path)
		if err != nil || string(data) != want {
			t.Fatalf("file %d=%q, %v", i, data, err)
		}
	}
}
func TestUploadRefusesOutOfOrderOversizedAndDuplicateFiles(t *testing.T) {
	for _, name := range []string{"out of order", "oversized", "duplicate", "dot"} {
		t.Run(name, func(t *testing.T) {
			request := &browserop.LiveViewerMessageUpload{Files: []browserop.UploadFile{{Name: "first", Size: 1}}}
			if name == "duplicate" {
				request.Files = append(request.Files, request.Files[0])
			}
			if name == "dot" {
				request.Files[0].Name = ".."
			}
			u, err := prepareUpload(t.TempDir(), request)
			if u != nil {
				defer u.close()
			}
			if name == "out of order" || name == "oversized" {
				if err != nil {
					t.Fatal(err)
				}
				if name == "out of order" {
					err = u.receive(1, []byte("x"))
				} else {
					err = u.receive(0, []byte("xx"))
				}
			}
			var failure *cdp.BrowserError
			if !errors.As(err, &failure) || failure.Kind != cdp.KindConfiguration {
				t.Fatalf("refusal=%v", err)
			}
		})
	}
}
