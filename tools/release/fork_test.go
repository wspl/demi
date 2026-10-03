package main

import (
	"crypto/rand"
	"fmt"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/artifacts/artifactstest"
	"golang.org/x/mod/module"
	"golang.org/x/mod/sumdb"
	"golang.org/x/mod/sumdb/dirhash"
	"golang.org/x/mod/sumdb/note"
)

func TestComparisonNamesEachChangeAndCountsWhatIsLeftOut(t *testing.T) {
	root := t.TempDir()
	selected := module.Version{Path: "example.test/shell", Version: "v1.0.0"}
	prefix := selected.Path + "@" + selected.Version + "/"
	upstream := map[string][]byte{prefix + "go.mod": []byte("module example.test/shell\n"), prefix + "same.go": []byte("same\n"), prefix + "change.go": []byte("first\nold\nlast\n"), prefix + "gone.go": []byte("gone\n"), prefix + ".github/test.yml": []byte("CI\n"), prefix + "testdata/upstream.txt": []byte("fixture\n")}
	archive := artifactstest.Zip(t, upstream)
	path := filepath.Join(root, "module.zip")
	writeFixture(t, path, archive)
	hash, err := dirhash.HashZip(path, dirhash.Hash1)
	if err != nil {
		t.Fatal(err)
	}
	proxy := artifactstest.Start(t, map[string]artifactstest.Answer{"/example.test/shell/@v/v1.0.0.zip": artifactstest.OK(archive)})
	signer, key, err := note.GenerateKey(rand.Reader, "fixture.test")
	if err != nil {
		t.Fatal(err)
	}
	for _, valid := range []bool{true, false} {
		t.Run(fmt.Sprint(valid), func(t *testing.T) {
			recorded := hash
			if !valid {
				recorded = "h1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
			}
			server := httptest.NewServer(sumdb.NewServer(sumdb.NewTestServer(signer, func(path, version string) ([]byte, error) {
				return []byte(path + " " + version + " " + recorded + "\n"), nil
			})))
			defer server.Close()
			client := artifacts.NewClientAllowingHTTP()
			defer client.Close()
			database := sumdb.NewClient(&checksumOps{ctx: t.Context(), client: client, url: server.URL, key: key})
			extracted, err := moduleUpstream(t.Context(), client, selected, t.TempDir(), proxy.URL(""), database)
			if !valid {
				if err == nil {
					t.Fatal("checksum mismatch accepted")
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			fork := t.TempDir()
			for name, data := range map[string][]byte{"go.mod": upstream[prefix+"go.mod"], "same.go": []byte("same\n"), "change.go": []byte("first\nnew\nlast\n"), "added.go": []byte("added\n"), "demi.patch": []byte("not compared")} {
				writeFixture(t, filepath.Join(fork, name), data)
			}
			summary, patch, err := compareFork(t.Context(), extracted, fork)
			if err != nil {
				t.Fatal(err)
			}
			want := "1 changed, 1 added, 1 removed, 2 left out\n  added added.go (+1 -0)\n  changed change.go (+1 -1)\n  removed gone.go (+0 -1)\n"
			if summary != want {
				t.Fatalf("summary: %q, want %q", summary, want)
			}

			if strings.Contains(patch, "same.go") || strings.Contains(patch, "demi.patch") || !strings.Contains(patch, "-old\n+new\n") {
				t.Fatal(patch)
			}
		})
	}
}
