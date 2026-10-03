package commandwire_test

import (
	"bytes"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"reflect"
	"strings"
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
		if len(commandwire.Targets) != len(p.Targets) {
			t.Fatal("publication catalog does not cover the Rust release fixture")
		}
		for i, target := range commandwire.Targets {
			artifact, err := p.TargetArtifact(commandwire.TargetTriple(target))
			if err != nil || artifact.Size != uint64(12345+i) {
				t.Fatalf("publication slot %d (%s): artifact = %+v, %v", i, target, artifact, err)
			}
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
	p := commandwire.PackageDescriptor{
		ID:              "demi.browser",
		Version:         "1",
		ProtocolVersion: 1,
		Operations:      []string{"open"},
		Targets:         map[string]commandwire.PackageArtifact{"aarch64-apple-darwin": {SHA256: hash, Size: 123}},
		Resources:       resources,
	}
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
			p.Resources = map[string]commandwire.PackageResource{"Bad Name": resources["chrome"]}
		},
		func(p *commandwire.PackageDescriptor) {
			p.Resources = map[string]commandwire.PackageResource{
				"chrome": {
					Title:   "Chrome",
					Targets: map[string]commandwire.ResourceArtifact{},
				},
			}
		},
	} {
		bad := p
		mutate(&bad)
		if err := bad.Validate(); err == nil {
			t.Fatal("accepted invalid descriptor")
		}
	}
}

func TestArtifactLocationRustWire(t *testing.T) {
	for _, tc := range []struct {
		name  string
		wire  string
		value commandwire.ArtifactLocation
	}{
		{"path", `{"path":"/tmp/program"}`, &commandwire.ArtifactPath{Path: "/tmp/program"}},
		{"url", `{"url":"https://example.com/program"}`, &commandwire.ArtifactURL{URL: "https://example.com/program"}},
		{
			"expiring URL",
			`{"url":"https://example.com/program","expiresAt":123}`,
			&commandwire.ArtifactURL{
				URL:       "https://example.com/program",
				ExpiresAt: new(int64(123)),
			},
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			got, err := commandwire.DecodeArtifactLocation([]byte(tc.wire))
			if err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(got, tc.value) {
				t.Fatalf("location = %#v; want %#v", got, tc.value)
			}
			for _, value := range []any{got, commandwire.ArtifactLocationJSON{Value: got}} {
				encoded, err := contract.EncodeJSON(value)
				if err != nil {
					t.Fatal(err)
				}
				if string(encoded) != tc.wire {
					t.Fatalf("encoded location = %s; want %s", encoded, tc.wire)
				}
			}
		})
	}
	for _, wire := range []string{
		`{}`,
		`{"path":"/tmp/program","url":"https://example.com/program"}`,
		`{"kind":"path","path":"/tmp/program"}`,
		`{"url":"https://example.com/program","expiresAt":null}`,
		`{"path":""}`,
		`{"url":"file:///tmp/program"}`,
	} {
		if _, err := commandwire.DecodeArtifactLocation([]byte(wire)); err == nil {
			t.Fatalf("accepted invalid location %s", wire)
		}
	}
}

func TestRustDescriptorEmptyResources(t *testing.T) {
	data, err := os.ReadFile("testdata/manifest.json")
	if err != nil {
		t.Fatal(err)
	}
	manifest, err := contract.Object(data)
	if err != nil {
		t.Fatal(err)
	}
	packages, err := contract.Object(manifest["packages"])
	if err != nil {
		t.Fatal(err)
	}
	for want, data := range packages {
		object, err := contract.Object(data)
		if err != nil {
			t.Fatal(err)
		}
		if _, ok := object["resources"]; ok {
			t.Fatal("fixture must have no resources")
		}
		for _, form := range []string{"absent", "empty", "null"} {
			t.Run(form, func(t *testing.T) {
				switch form {
				case "absent":
					delete(object, "resources")
				case "empty":
					object["resources"] = json.RawMessage(`{}`)
				case "null":
					object["resources"] = json.RawMessage(`null`)
				}
				wire, err := contract.EncodeJSON(object)
				if err != nil {
					t.Fatal(err)
				}
				p, err := commandwire.DecodePackageDescriptor(wire)
				if form == "null" {
					if err == nil {
						t.Fatal("accepted null resources")
					}
					return
				}
				if err != nil {
					t.Fatal(err)
				}
				got, err := p.Digest()
				if err != nil || got != want {
					t.Fatalf("descriptor digest = %s, %v; want Rust hash %s", got, err, want)
				}
				encoded, err := contract.EncodeJSON(p)
				if err != nil {
					t.Fatal(err)
				}
				fields, err := contract.Object(encoded)
				if err != nil {
					t.Fatal(err)
				}
				if _, ok := fields["resources"]; ok {
					t.Fatal("empty resources written to descriptor")
				}
			})
		}
	}
}

// These APIs are the checks the backend install route and release manifests share.
func TestArtifactIdentityChecks(t *testing.T) {
	for _, tc := range []struct {
		name  string
		value string
		valid bool
	}{
		{"valid", strings.Repeat("0123456789abcdef", 4), true},
		{"empty", "", false},
		{"short", strings.Repeat("0", 63), false},
		{"long", strings.Repeat("0", 65), false},
		{"uppercase", strings.Repeat("A", 64), false},
		{"nonhex", strings.Repeat("g", 64), false},
		{"unicode", strings.Repeat("é", 32), false},
		{"newline", strings.Repeat("0", 63) + "\n", false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if commandwire.IsDigest(tc.value) != tc.valid {
				t.Fatal("incorrect digest predicate")
			}
			if err := commandwire.Digest(tc.value); (err == nil) != tc.valid {
				t.Fatalf("Digest(%q): %v", tc.value, err)
			}
		})
	}
	for _, target := range commandwire.Targets {
		if !commandwire.IsTarget(target) {
			t.Fatalf("published target %s rejected", target)
		}
		if err := commandwire.TargetTriple(target).Validate(); err != nil {
			t.Fatal(err)
		}
	}
	for _, target := range []string{
		"",
		"amd64",
		"x86_64-unknown-linux-gnu",
		"aarch64-apple-darwin\n",
		"AARCH64-APPLE-DARWIN",
	} {
		if commandwire.IsTarget(target) {
			t.Fatalf("invalid target %q accepted", target)
		}
		if err := commandwire.TargetTriple(target).Validate(); err == nil {
			t.Fatalf("invalid target triple %q accepted", target)
		}
	}
}

func TestReleaseTargetArtifacts(t *testing.T) {
	valid := commandwire.PackageArtifact{SHA256: strings.Repeat("a", 64), Size: 1}
	for _, tc := range []struct {
		name    string
		targets map[string]commandwire.PackageArtifact
		valid   bool
	}{
		{"nil development release", nil, true},
		{"empty development release", map[string]commandwire.PackageArtifact{}, true},
		{"known target", map[string]commandwire.PackageArtifact{"aarch64-apple-darwin": valid}, true},
		{"unknown target", map[string]commandwire.PackageArtifact{"unknown": valid}, false},
		{
			"invalid digest",
			map[string]commandwire.PackageArtifact{
				"aarch64-apple-darwin": {
					SHA256: "invalid",
					Size:   1,
				},
			},
			false,
		},
		{"zero size", map[string]commandwire.PackageArtifact{"aarch64-apple-darwin": {SHA256: valid.SHA256, Size: 0}}, false},
		{
			"maximum size",
			map[string]commandwire.PackageArtifact{
				"aarch64-apple-darwin": {
					SHA256: valid.SHA256,
					Size:   9007199254740991,
				},
			},
			true,
		},
		{
			"oversized artifact",
			map[string]commandwire.PackageArtifact{
				"aarch64-apple-darwin": {
					SHA256: valid.SHA256,
					Size:   9007199254740992,
				},
			},
			false,
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			err := commandwire.TargetArtifacts(tc.targets)
			if (err == nil) != tc.valid {
				t.Fatalf("target artifacts: %v; want valid=%v", err, tc.valid)
			}
			if !tc.valid {
				var field *contract.Error
				if !errors.As(err, &field) || field.Path == "" {
					t.Fatalf("missing artifact error path: %v", err)
				}
			}
		})
	}
}

// TestRunnerMessagePackCorpus pins commandwire's embedded values to the
// rmp_serde::to_vec_named bytes used by the runner. It needs only fixture IO
// and takes less than a second; parent encodings include nested wire values.
func TestRunnerMessagePackCorpus(t *testing.T) {
	for _, tc := range []struct {
		fixture   string
		path      string
		roundTrip func([]byte) ([]byte, error)
	}{
		{"backend-to-runner/artifact_location.path", "location", runnerRoundTrip(commandwire.DecodeArtifactLocationMsgpack)},
		{"backend-to-runner/artifact_location.url", "location", runnerRoundTrip(commandwire.DecodeArtifactLocationMsgpack)},
		{"backend-to-runner/job_start", "context", runnerRoundTrip(commandwire.DecodeCommandContextMsgpack)},
		{"backend-to-runner/job_start.minimal", "context", runnerRoundTrip(commandwire.DecodeCommandContextMsgpack)},
		{"backend-to-runner/service_open", "context", runnerRoundTrip(commandwire.DecodeCommandContextMsgpack)},
		{"backend-to-runner/service_open.minimal", "context", runnerRoundTrip(commandwire.DecodeCommandContextMsgpack)},
		{"backend-to-runner/service_open", "package", runnerRoundTrip(commandwire.DecodePackageDescriptorMsgpack)},
		{"backend-to-runner/service_open.minimal", "package", runnerRoundTrip(commandwire.DecodePackageDescriptorMsgpack)},
		{"runner-to-backend/job_exit", "files.[].kind", runnerRoundTrip(commandwire.DecodeEditKindMsgpack)},
		{"runner-to-backend/job_exit", "files.[].edits.[]", runnerRoundTrip(commandwire.DecodeEditCopiesMsgpack)},
		{"runner-to-backend/numbers_reserve", "sequence", runnerRoundTrip(commandwire.DecodeServiceSequenceMsgpack)},
	} {
		t.Run(tc.fixture+"/"+tc.path, func(t *testing.T) {
			data, err := os.ReadFile("testdata/runner/" + tc.fixture + ".msgpack")
			if err != nil {
				t.Fatal(err)
			}
			parts := runnerParts(t, data, strings.Split(tc.path, "."))
			if len(parts) == 0 {
				t.Fatal("fixture contains no selected commandwire values")
			}
			for i, want := range parts {
				got, err := tc.roundTrip(want)
				if err != nil {
					t.Fatalf("part %d: %v", i, err)
				}
				if !bytes.Equal(got, want) {
					t.Errorf("part %d: MessagePack = %x; Rust wrote %x", i, got, want)
				}
			}
		})
	}
}

// runnerRoundTrip uses a generated commandwire decoder before re-encoding
// the embedded value with its generated MessagePack method.
func runnerRoundTrip[T any](decode func([]byte) (T, error)) func([]byte) ([]byte, error) {
	return func(data []byte) ([]byte, error) {
		value, err := decode(data)
		if err != nil {
			return nil, err
		}
		return contract.EncodeMsgpack(value)
	}
}

// runnerParts extracts commandwire values without declaring the runner's
// envelope types again or re-encoding the oracle bytes. [] selects all array
// entries.
func runnerParts(t *testing.T, data []byte, path []string) [][]byte {
	t.Helper()
	if len(path) == 0 {
		return [][]byte{data}
	}
	var children [][]byte
	if path[0] == "[]" {
		var err error
		children, err = contract.MsgpackList(data, func(raw []byte) ([]byte, error) { return raw, nil })
		if err != nil {
			t.Fatal(err)
		}
	} else {
		fields, err := contract.MsgpackObject(data)
		if err != nil {
			t.Fatal(err)
		}
		raw, ok := fields[path[0]]
		if !ok {
			t.Fatalf("fixture is missing %s", path[0])
		}
		children = append(children, raw)
	}
	var parts [][]byte
	for _, child := range children {
		parts = append(parts, runnerParts(t, child, path[1:])...)
	}
	return parts
}
