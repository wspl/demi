package commandwire_test

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"testing"

	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/contract"
)

// These in-memory contract scenarios need no services or wall-clock waits.
func TestRustManifestDigests(t *testing.T) {
	data, err := os.ReadFile("testdata/manifest.json")
	if err != nil {
		t.Fatal(err)
	}
	manifest, err := contract.Object(data)
	if err != nil {
		t.Fatal(err)
	}
	want, err := contract.Decode[string](manifest["hash"])
	if err != nil {
		t.Fatal(err)
	}
	delete(manifest, "hash")
	got, err := commandwire.CanonicalDigest(manifest)
	if err != nil || got != want {
		t.Fatalf("manifest digest = %s, %v; want %s", got, err, want)
	}
	packages, err := contract.Object(manifest["packages"])
	if err != nil {
		t.Fatal(err)
	}
	for want, data := range packages {
		p, err := commandwire.DecodePackageDescriptor(data)
		if err != nil {
			t.Fatal(err)
		}
		got, err := p.Digest()
		if err != nil || got != want {
			t.Fatalf("descriptor digest = %s, %v; want %s", got, err, want)
		}
		artifact, err := p.TargetArtifact("aarch64-apple-darwin")
		if err != nil || artifact.Size != 12345 {
			t.Fatalf("selected artifact = %+v, %v", artifact, err)
		}
		if !p.Serves(commandwire.ServiceInfo{ProtocolVersion: 1, Operations: []string{"fixture.echo", "file.read"}}) {
			t.Fatal("catalog order changed compatibility")
		}
		if p.Serves(commandwire.ServiceInfo{ProtocolVersion: 1, Operations: []string{"file.read"}}) {
			t.Fatal("incomplete service catalog accepted")
		}
		if got, ok := p.Carries("aarch64-apple-darwin", artifact.SHA256); !ok || got != artifact {
			t.Fatal("pinned executable not found")
		}
	}
}

func TestCanonicalDigestJCS(t *testing.T) {
	for _, tc := range []struct{ name, input, canonical string }{
		{"binary64", `{"b":333333333.33333329,"a":1e30,"c":-0}`, `{"a":1e+30,"b":333333333.3333333,"c":0}`},
		{"UTF16 order", `{"\ue000":1,"😀":2}`, `{"😀":2,"":1}`},
		{"escaping", `{"x":"<>&\u000a"}`, `{"x":"<>&\n"}`},
	} {
		t.Run(tc.name, func(t *testing.T) {
			got, err := commandwire.CanonicalDigest(json.RawMessage(tc.input))
			want := fmt.Sprintf("%x", sha256.Sum256([]byte(tc.canonical)))
			if err != nil || got != want {
				t.Fatalf("digest = %s, %v; want %s", got, err, want)
			}
		})
	}
	for _, bad := range []string{`{"a":1,"a":2}`, `"\ud800"`, `{} {}`} {
		if _, err := commandwire.CanonicalDigest(json.RawMessage(bad)); err == nil {
			t.Fatalf("accepted corrupt JSON %s", bad)
		}
	}
}

func TestResourceSelectionAndDescriptorValidation(t *testing.T) {
	hash := fmt.Sprintf("%064d", 0)
	resourceHash := fmt.Sprintf("%064d", 1)
	resources := map[string]commandwire.PackageResource{
		"chrome": {Title: "Chrome", Targets: map[string]commandwire.ResourceArtifact{
			"aarch64-apple-darwin": {SHA256: resourceHash, Size: 456, Entry: "bin/chrome"},
		}},
	}
	p := commandwire.PackageDescriptor{ID: "demi.browser", Version: "1", ProtocolVersion: 1, Operations: []string{"open"}, Targets: map[string]commandwire.PackageArtifact{"aarch64-apple-darwin": {SHA256: hash, Size: 123}}, Resources: &resources}
	if err := p.Validate(); err != nil {
		t.Fatal(err)
	}
	a, ok := p.Carries("aarch64-apple-darwin", resourceHash)
	if !ok || a.Size != 456 {
		t.Fatalf("resource archive = %+v, %v", a, ok)
	}
	if _, ok := p.Carries("x86_64-apple-darwin", resourceHash); ok {
		t.Fatal("resource selected for wrong target")
	}
	for _, mutate := range []func(*commandwire.PackageDescriptor){
		func(p *commandwire.PackageDescriptor) { p.ID = "invalid" },
		func(p *commandwire.PackageDescriptor) { p.ProtocolVersion = 2 },
		func(p *commandwire.PackageDescriptor) { p.Operations = []string{"open", "open"} },
		func(p *commandwire.PackageDescriptor) { p.Operations = []string{""} },
		func(p *commandwire.PackageDescriptor) {
			p.Targets = map[string]commandwire.PackageArtifact{"wrong": {SHA256: hash, Size: 1}}
		},
		func(p *commandwire.PackageDescriptor) {
			p.Resources = &map[string]commandwire.PackageResource{"Bad Name": resources["chrome"]}
		},
		func(p *commandwire.PackageDescriptor) {
			p.Resources = &map[string]commandwire.PackageResource{"chrome": {Title: "Chrome", Targets: map[string]commandwire.ResourceArtifact{}}}
		},
	} {
		bad := p
		mutate(&bad)
		if err := bad.Validate(); err == nil {
			t.Fatal("accepted invalid descriptor")
		}
	}
}
