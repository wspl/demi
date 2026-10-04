package providerhost_test

import (
	"context"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync"
	"sync/atomic"
	"testing"

	"github.com/google/go-cmp/cmp"
	"github.com/wspl/demi/internal/backend/providerhost"
	"github.com/wspl/demi/internal/commandpackage/claudecode/claudecodeproto"
)

func TestNewestReleaseUsesPointerManifestAndCache(t *testing.T) {
	var mode atomic.Int32
	var requests atomic.Int32
	started := make(chan struct{})
	answer := make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		requests.Add(1)
		if r.URL.Path == "/r/latest" {
			switch mode.Load() {
			case 0:
				select {
				case <-started:
				default:
					close(started)
				}
				<-answer
				_, _ = fmt.Fprint(w, "2.1.3\n")
			case 1:
				_, _ = fmt.Fprint(w, "2.1.3")
			case 2:
				_, _ = fmt.Fprint(w, "latest-and-greatest")
			case 3:
				_, _ = fmt.Fprint(w, "2.1.4")
			case 4:
				_, _ = fmt.Fprint(w, "2.1.5")
			case 5:
				w.Header().Set("Location", "/r/moved")
				w.WriteHeader(http.StatusFound)
			}
			return
		}
		if r.URL.Path == "/r/2.1.4/manifest.json" {
			http.NotFound(w, r)
			return
		}
		version := "2.1.3"
		if r.URL.Path == "/r/2.1.5/manifest.json" {
			version = "2.1.4"
		}
		_, _ = fmt.Fprintf(
			w,
			`{"version":%q,"commit":"ignored",`+
				`"platforms":{"linux-x64":{"binary":"claude","checksum":%q,"size":1024},`+
				`"win32-x64":{"binary":"claude.exe","checksum":%q,"size":2048}}}`,
			version,
			strings.Repeat("ab", 32),
			strings.Repeat("cd", 32),
		)
	}))
	defer server.Close()
	base, err := url.Parse(server.URL + "/r")
	if err != nil {
		t.Fatal(err)
	}
	releases, err := providerhost.NewClaudeReleases(base)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := releases.Close(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	var workers sync.WaitGroup
	values := make([]claudecodeproto.Release, 2)
	for i := range 2 {
		workers.Go(func() {
			value, err := releases.Latest(t.Context(), false)
			if err != nil {
				t.Error(err)
			}
			values[i] = value
		})
	}
	<-started
	close(answer)
	workers.Wait()
	first := values[0]
	if diff := cmp.Diff(first, values[1]); diff != "" {
		t.Fatal(diff)
	}
	if first.Version != "2.1.3" || requests.Load() != 2 {
		t.Fatal(first, requests.Load())
	}
	linux := first.Platforms["linux-x64"]
	if linux.URL != server.URL+"/r/2.1.3/linux-x64/claude" || linux.Size != 1024 ||
		linux.SHA256 != strings.Repeat("ab", 32) ||
		first.Platforms["win32-x64"].URL != server.URL+"/r/2.1.3/win32-x64/claude.exe" {
		t.Fatal(first)
	}
	mode.Store(1)
	next, err := releases.Latest(t.Context(), true)
	if err != nil {
		t.Fatal(err)
	}
	if diff := cmp.Diff(first, next); diff != "" {
		t.Fatal(diff)
	}
	if requests.Load() != 3 {
		t.Fatal("manifest fetched twice")
	}
	for _, c := range []struct {
		mode int32
		want string
	}{
		{2, "the distribution named no version"},
		{3, "the release of version 2.1.4 could not be read " +
			"(the distribution answered 404 Not Found)"},
		{4, "the release of version 2.1.5 could not be read " +
			"(its manifest names version 2.1.4)"},
		{5, "the distribution answered 302 Found"},
	} {
		mode.Store(c.mode)
		if _, err := releases.Latest(t.Context(), true); err == nil || err.Error() != c.want {
			t.Fatalf("mode %d: %v", c.mode, err)
		}
	}
	count := requests.Load()
	kept, err := releases.Latest(t.Context(), false)
	if err != nil {
		t.Fatal(err)
	}
	if diff := cmp.Diff(first, kept); diff != "" {
		t.Fatal(diff)
	}
	if requests.Load() != count {
		t.Fatal("failure forgot good release")
	}
}
