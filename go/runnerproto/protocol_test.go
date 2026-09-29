package runnerproto_test

import (
	"bytes"
	"maps"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandtree"
	"github.com/wspl/demi/go/runnerproto"
)

// These contract tables run in under one second, without processes or timers.
func TestManifestVerifiesAndBuildsTheRustCorpus(t *testing.T) {
	data, err := os.ReadFile("testdata/manifest.json")
	if err != nil {
		t.Fatal(err)
	}
	manifest, err := runnerproto.DecodeManifest(data)
	if err != nil {
		t.Fatal(err)
	}
	roots := []commandtree.Node{}
	for _, root := range manifest.Roots {
		roots = append(roots, root.Tree)
	}
	packages := slices.Collect(maps.Values(manifest.Packages))
	built, err := runnerproto.BuildManifest(roots, packages)
	if err != nil || built.Hash != manifest.Hash {
		t.Fatalf("build: %s, %v, want %s", built.Hash, err, manifest.Hash)
	}
	encoded, err := built.Encode()
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(encoded, data) {
		t.Fatal("manifest bytes differ from Rust")
	}
	parsed, err := runnerproto.DecodeManifest(encoded)
	if err != nil || parsed.Hash != manifest.Hash {
		t.Fatalf("encoded manifest: %v", err)
	}
	for _, change := range [][2]string{
		{`"summary":"CLI fixture."`, `"summary":"corrupt"`},
		{`"operations":["file.read","fixture.echo"]`, `"operations":["file.read","file.read"]`},
		{`"positionals":["path"]`, `"positionals":["path","body"]`},
		{`"operation":"file.read"`, `"operation":"file.gone"`},
		{`"fixture":{"tree"`, `"other":{"tree"`},
	} {
		changed := bytes.Replace(data, []byte(change[0]), []byte(change[1]), 1)
		if bytes.Equal(changed, data) {
			t.Fatalf("mutation did not apply: %s", change[0])
		}
		if _, err := runnerproto.DecodeManifest(changed); err == nil {
			t.Fatalf("accepted %s", change[1])
		}
	}
	native := func(pkg, op string) commandtree.Node {
		return commandtree.Leaf{Name: "native", Summary: "Native", Kind: commandtree.KindNative, Binding: &commandtree.Binding{Package: pkg, Operation: op}}
	}
	rpc := commandtree.Leaf{Name: "rpc", Summary: "RPC", Kind: commandtree.KindRPC}
	cases := []struct {
		roots    []commandtree.Node
		packages []commandservice.PackageDescriptor
		want     string
	}{
		{[]commandtree.Node{native("demicodes.other", "file.read")}, packages, "not configured"},
		{[]commandtree.Node{native("demicodes.fixture", "file.gone")}, packages, "no operation"},
		{[]commandtree.Node{rpc, rpc}, nil, "duplicate root"},
		{nil, append(slices.Clone(packages), packages...), "duplicate native package"},
	}
	for _, c := range cases {
		if _, err := runnerproto.BuildManifest(c.roots, c.packages); err == nil || !strings.Contains(err.Error(), c.want) {
			t.Errorf("%v, want %s", err, c.want)
		}
	}
	exportCorpus(t, "manifest.json", encoded)
}

func TestReleaseChecksEveryField(t *testing.T) {
	data, err := os.ReadFile("testdata/release.json")
	if err != nil {
		t.Fatal(err)
	}
	record, err := runnerproto.DecodeRunnerRelease(data)
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := record.Encode()
	if err != nil || !bytes.Equal(encoded, data) {
		t.Fatalf("release bytes differ from Rust: %s %v", encoded, err)
	}
	for _, change := range [][2]string{
		{strings.Repeat("a", 64), strings.Repeat("A", 64)},
		{`"wire":24`, `"wire":23`},
		{`"commandProtocol":1`, `"commandProtocol":2`},
		{"aarch64-apple-darwin", "aarch64-apple-ios"},
		{`"size":1`, `"size":0`},
		{`"wire":24`, `"wire":24,"channel":"beta"`},
	} {
		changed := bytes.Replace(data, []byte(change[0]), []byte(change[1]), 1)
		if bytes.Equal(data, changed) {
			t.Fatalf("mutation did not apply: %s", change[0])
		}
		if _, err := runnerproto.DecodeRunnerRelease(changed); err == nil {
			t.Fatalf("accepted %s", change[1])
		}
	}
	exportCorpus(t, "release.json", encoded)
}

func TestKeptRecordsNamedFormAndRefuseBrokenFiles(t *testing.T) {
	data, err := os.ReadFile("testdata/kept/output.msgpack")
	if err != nil {
		t.Fatal(err)
	}
	records, err := runnerproto.DecodeRecords(data)
	if err != nil {
		t.Fatal(err)
	}
	var encoded []byte
	for _, record := range records {
		raw, err := runnerproto.EncodeRecord(record)
		if err != nil {
			t.Fatal(err)
		}
		encoded = append(encoded, raw...)
	}
	if !bytes.Equal(encoded, data) {
		t.Fatal("kept record bytes differ from named corpus")
	}
	gap, err := runnerproto.EncodeRecord(runnerproto.KeptLeftOut{Bytes: ^uint64(0)})
	if err != nil {
		t.Fatal(err)
	}
	if len(gap) > 32 {
		t.Fatal("gap exceeds the kept-file read allowance")
	}
	empty, err := runnerproto.EncodeRecord(runnerproto.KeptOutput{Stream: runnerproto.OutputStreamStdout})
	if err != nil {
		t.Fatal(err)
	}
	large, err := runnerproto.EncodeRecord(runnerproto.KeptOutput{Stream: runnerproto.OutputStreamStdout, Bytes: make([]byte, runnerproto.JobKeptBytes)})
	if err != nil {
		t.Fatal(err)
	}
	for _, bad := range [][]byte{append(slices.Clone(gap), gap...), data[:len(data)-1], empty, large} {
		if _, err := runnerproto.DecodeRecords(bad); err == nil {
			t.Fatal("accepted a broken kept file")
		}
	}
	exportCorpus(t, "kept/output.msgpack", encoded)
}

// exportCorpus is used only for the manual Rust reverse-direction check.
func exportCorpus(t *testing.T, name string, data []byte) {
	t.Helper()
	dir := os.Getenv("DEMI_RUNNER_CORPUS_OUTPUT")
	if dir == "" {
		return
	}
	path := filepath.Join(dir, name)
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, data, 0644); err != nil {
		t.Fatal(err)
	}
}

func TestManifestHashCoversDeclarationsAndPackages(t *testing.T) {
	rpc := commandtree.Leaf{Name: "rpc", Summary: "RPC", Kind: commandtree.KindRPC}
	first, err := runnerproto.BuildManifest([]commandtree.Node{rpc}, nil)
	if err != nil {
		t.Fatal(err)
	}
	hint := "working"
	rpc.RunningHint = &hint
	second, err := runnerproto.BuildManifest([]commandtree.Node{rpc}, nil)
	if err != nil {
		t.Fatal(err)
	}
	if first.Hash == second.Hash {
		t.Fatal("running hint not covered by hash")
	}
	data, err := os.ReadFile("testdata/manifest.json")
	if err != nil {
		t.Fatal(err)
	}
	// Package identity changes are also covered by rebuilding with a changed version.
	original, err := runnerproto.DecodeManifest(data)
	if err != nil {
		t.Fatal(err)
	}
	packages := slices.Collect(maps.Values(original.Packages))
	a, err := runnerproto.BuildManifest([]commandtree.Node{rpc}, packages)
	if err != nil {
		t.Fatal(err)
	}
	packages[0].Version = "next"
	b, err := runnerproto.BuildManifest([]commandtree.Node{rpc}, packages)
	if err != nil {
		t.Fatal(err)
	}
	if a.Hash == b.Hash {
		t.Fatal("package version not covered by hash")
	}
}

func TestBootCorpusMatchesRustInBothFormats(t *testing.T) {
	for _, format := range []string{"json", "msgpack"} {
		data, err := os.ReadFile("testdata/boot." + format)
		if err != nil {
			t.Fatal(err)
		}
		var encoded []byte
		if format == "json" {
			record, err := runnerproto.DecodeManagedBoot(data)
			if err != nil {
				t.Fatal(err)
			}
			encoded, err = record.Encode()
			if err != nil {
				t.Fatal(err)
			}
		} else {
			record, err := runnerproto.DecodeManagedBootMsgpack(data)
			if err != nil {
				t.Fatal(err)
			}
			encoded, err = record.MarshalMsgpack()
			if err != nil {
				t.Fatal(err)
			}
		}
		if !bytes.Equal(data, encoded) {
			t.Fatalf("boot %s differs from Rust", format)
		}
		exportCorpus(t, "boot."+format, encoded)
	}
}
