package skills

import (
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/go-git/go-git/v5/plumbing"
	"github.com/go-git/go-git/v5/plumbing/protocol/packp"
	"github.com/go-git/go-git/v5/plumbing/protocol/packp/capability"
)

// A malformed upload must release its response without waiting for instance
// shutdown. Synchronization uses the server's request cancellation event.
func TestMalformedUploadClosesResponse(t *testing.T) {
	ended := make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if _, err := io.Copy(io.Discard, r.Body); err != nil {
			t.Error(err)
			return
		}
		if strings.HasSuffix(r.URL.Path, "/info/refs") {
			head := plumbing.NewHash(strings.Repeat("a", 40))
			refs := packp.NewAdvRefs()
			refs.Head = &head
			refs.References["refs/heads/main"] = head
			refs.Prefix = [][]byte{[]byte("# service=git-upload-pack\n"), {}}
			if err := refs.Capabilities.Set(capability.Shallow); err != nil {
				t.Error(err)
				return
			}
			w.Header().Set("Content-Type", "application/x-git-upload-pack-advertisement")
			if err := refs.Encode(w); err != nil {
				t.Error(err)
			}
			return
		}
		w.Header().Set("Content-Type", "application/x-git-upload-pack-result")
		if _, err := io.WriteString(w, "0000000cNOT-ACK\n"); err != nil {
			t.Error(err)
			return
		}
		if err := http.NewResponseController(w).Flush(); err != nil {
			t.Error(err)
			return
		}
		<-r.Context().Done()
		close(ended)
	}))
	defer server.Close()
	if _, err := fetch(t.Context(), server.URL+"/repo", "repo"); err == nil {
		t.Fatal("accepted malformed upload")
	}
	<-ended
}
