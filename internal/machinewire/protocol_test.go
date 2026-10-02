package machinewire_test

import (
	"bytes"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/machinewire"
	"github.com/wspl/demi/internal/runnerwire"
)

// These in-memory boundary tests have a one-second budget and use no external resources.
func TestCorpus(t *testing.T) {
	for _, direction := range []string{"backend-to-manager", "manager-to-backend"} {
		paths, err := filepath.Glob("testdata/" + direction + "/*.ndjson")
		if err != nil || len(paths) == 0 {
			t.Fatalf("corpus: %v", err)
		}
		for _, path := range paths {
			t.Run(path, func(t *testing.T) {
				data, err := os.ReadFile(path)
				if err != nil {
					t.Fatal(err)
				}
				var msg machinewire.Message
				if direction == "backend-to-manager" {
					request, err := machinewire.DecodeRequest(bytes.TrimSuffix(data, []byte{'\n'}))
					if err != nil {
						t.Fatal(err)
					}
					if request.Call.Name() != strings.TrimSuffix(filepath.Base(path), ".ndjson") {
						t.Fatal("operation name differs from fixture")
					}
					msg = request
				} else {
					msg, err = machinewire.DecodeResponse(bytes.TrimSuffix(data, []byte{'\n'}))
					if err != nil {
						t.Fatal(err)
					}
				}
				encoded, err := machinewire.EncodeLine(msg)
				if err != nil {
					t.Fatal(err)
				}
				if !bytes.Equal(encoded, data) {
					t.Fatalf("got %s\nwant %s", encoded, data)
				}
			})
		}
	}
}

// output reads a corpus result through the schema selected by the outstanding operation.
func output[T any](t *testing.T, name string, op machinewire.Operation[T]) T {
	t.Helper()
	data, err := os.ReadFile("testdata/manager-to-backend/ok." + name + ".ndjson")
	if err != nil {
		t.Fatal(err)
	}
	response, err := machinewire.DecodeResponse(data)
	if err != nil {
		t.Fatal(err)
	}
	ok, success := response.(*machinewire.OK)
	if !success {
		t.Fatal("expected success response")
	}
	value, err := op.DecodeOutput(ok.Result)
	if err != nil {
		t.Fatal(err)
	}
	return value
}

func TestOperationResults(t *testing.T) {
	state := output(t, "image_state", machinewire.ImageStateParams{})
	if state == nil || state.Generation != "7d2c9b1a-5e4f-4a3b-8c2d-1e0f9a8b7c6d" || string(state.BaseVersion) != strings.Repeat("b", 64) || state.ResetID != nil || state.SystemBytes != 1<<30 || state.HomeBytes != 2<<30 {
		t.Fatalf("unexpected state: %+v", state)
	}
	reset := output(t, "image_state.reset", machinewire.ImageStateParams{})
	if reset == nil || reset.ResetID == nil || *reset.ResetID != "reset-4f1e" {
		t.Fatalf("unexpected reset: %+v", reset)
	}
	if output(t, "image_state.none", machinewire.ImageStateParams{}) != nil {
		t.Fatal("expected absent state")
	}
	if output(t, "runtime_state", machinewire.RuntimeStateParams{}) != machinewire.RuntimeStateRunning {
		t.Fatal("expected running")
	}
	if output(t, "runtime_state.stopped", machinewire.RuntimeStateParams{}) != machinewire.RuntimeStateStopped {
		t.Fatal("expected stopped")
	}
	if string(output(t, "current_base_version", machinewire.CurrentBaseVersionParams{})) != strings.Repeat("b", 64) {
		t.Fatal("base version")
	}
	output(t, "reconcile", machinewire.ReconcileParams{})
	for _, op := range []machinewire.Operation[machinewire.Unit]{machinewire.ReconcileParams{}, machinewire.WakeParams{}, machinewire.HibernateParams{}, machinewire.CheckpointParams{}, machinewire.GrowVolumeParams{}, machinewire.ResetParams{}} {
		if _, err := op.DecodeOutput([]byte("null")); err != nil {
			t.Fatal(err)
		}
		if _, err := op.DecodeOutput([]byte("{}")); err == nil {
			t.Fatal("accepted non-null unit result")
		}
	}
}

func TestRequestValidation(t *testing.T) {
	accepted := []string{
		`{"id":"1","op":"hibernate","params":{"deviceId":"dev-1"},"trace":"x"}`,
		`{"id":"1","op":"hibernate","params":{"deviceId":"dev-1","force":true}}`,
		`{"id":"1","op":"reconcile","params":{"all":true}}`,
		`{"id":"1","op":"wake","params":{"deviceId":"dev-1","boot":{"backendUrl":"http://backend","deviceToken":"opaque"}}}`,
		`{"id":"1","op":"reset","params":{"deviceId":"../x","operationId":"o","baseVersion":"../b"}}`,
	}
	for _, line := range accepted {
		if _, err := machinewire.DecodeRequest([]byte(line)); err != nil {
			t.Errorf("%s: %v", line, err)
		}
	}
	refused := []string{
		`not json`,
		`{"id":"1","op":"hibernate","params":{"deviceId":"dev-1"}} trailing`,
		`{"id":"1","op":"reboot","params":{}}`,
		`{"id":"1","op":"hibernate"}`,
		`{"op":"reconcile","params":{}}`,
		`{"id":"","op":"reconcile","params":{}}`,
		`{"id":"1","op":"hibernate","params":{"deviceId":""}}`,
		`{"id":"1","op":"hibernate","params":{"deviceId":7}}`,
		`{"id":"1","op":"wake","params":{"deviceId":"dev-1"}}`,
		`{"id":"1","op":"wake","params":{"deviceId":"dev-1","boot":{"backendUrl":"http://backend","deviceToken":"opaque","command":"x"}}}`,
		`{"id":"1","op":"wake","params":{"deviceId":"dev-1","boot":{"backendUrl":"ftp://backend","deviceToken":"opaque"}}}`,
		`{"id":"1","op":"wake","params":{"deviceId":"dev-1","boot":{"backendUrl":"http://backend","deviceToken":"two words"}}}`,
		`{"id":"1","op":"reset","params":{"deviceId":"dev-1","operationId":"","baseVersion":"b"}}`,
		`{"id":"1","op":"reset","params":{"deviceId":"dev-1","operationId":"o"}}`,
	}
	for _, size := range []string{"0", "-1", "1.5"} {
		refused = append(refused, `{"id":"1","op":"grow_volume","params":{"deviceId":"dev-1","volume":"home","bytes":`+size+`}}`)
	}
	refused = append(refused, `{"id":"1","op":"grow_volume","params":{"deviceId":"dev-1","volume":"swap","bytes":1}}`)
	for _, line := range refused {
		if _, err := machinewire.DecodeRequest([]byte(line)); err == nil {
			t.Errorf("accepted %s", line)
		}
	}
}

func TestResponseValidation(t *testing.T) {
	for _, line := range []string{`{"type":"done","id":"1"}`, `{"type":"ok","result":null}`, `{"type":"error","id":"1"}`, `{"type":"death","deviceId":""}`} {
		if _, err := machinewire.DecodeResponse([]byte(line)); err == nil {
			t.Errorf("accepted %s", line)
		}
	}
}

func TestImageState(t *testing.T) {
	valid := `{"generation":"g-1","baseVersion":"b_1","resetId":null,"systemBytes":1,"homeBytes":2}`
	state, err := machinewire.DecodeMachineImageState([]byte(valid))
	if err != nil {
		t.Fatal(err)
	}
	size, err := state.Bytes(machinewire.VolumeHome)
	if err != nil || size != 2 {
		t.Fatalf("capacity: %d %v", size, err)
	}
	for _, bad := range []string{
		strings.Replace(valid, `"resetId":null,`, "", 1),
		strings.Replace(valid, `g-1`, `g/1`, 1),
		strings.Replace(valid, `b_1`, "", 1),
		strings.Replace(valid, `"systemBytes":1`, `"systemBytes":0`, 1),
		strings.Replace(valid, `}`, `,"x":1}`, 1),
	} {
		if _, err := machinewire.DecodeMachineImageState([]byte(bad)); err == nil {
			t.Errorf("accepted %s", bad)
		}
	}
}

func TestImageNames(t *testing.T) {
	for _, name := range []string{"dev-1", "A_b-9", "0f6c3d4e-8a9b-4c1d-9e2f-3a4b5c6d7e8f"} {
		if _, err := machinewire.ParseDeviceID(name); err != nil {
			t.Fatal(err)
		}
	}
	for _, name := range []string{"", ".", "..", "a/b", "a b", "dev.1", "é"} {
		if _, err := machinewire.ParseDeviceID(name); err == nil {
			t.Errorf("accepted %q", name)
		}
	}
}

func TestBootCredentialRedaction(t *testing.T) {
	data, err := os.ReadFile("testdata/backend-to-manager/wake.ndjson")
	if err != nil {
		t.Fatal(err)
	}
	request, err := machinewire.DecodeRequest(data)
	if err != nil {
		t.Fatal(err)
	}
	for _, format := range []string{"%v", "%+v", "%#v"} {
		debug := fmt.Sprintf(format, request)
		if !strings.Contains(debug, "backend.example.com") || strings.Contains(debug, "dt_6b1f0c2e9a7d4c3b") {
			t.Fatalf("credential diagnostic: %s", debug)
		}
	}
}

// The wake boundary delegates URL normalization to the runner's boot codec.
func TestBootURLNormalization(t *testing.T) {
	input := []byte(`{"id":"1","op":"wake","params":{"deviceId":"dev-1","boot":{"backendUrl":"HTTP://BACKEND.EXAMPLE.COM:80","deviceToken":"opaque"}}}`)
	request, err := machinewire.DecodeRequest(input)
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := machinewire.EncodeLine(request)
	if err != nil {
		t.Fatal(err)
	}
	want := strings.Replace(string(input), "HTTP://BACKEND.EXAMPLE.COM:80", "http://backend.example.com/", 1) + "\n"
	if string(encoded) != want {
		t.Fatalf("got %s\nwant %s", encoded, want)
	}
}

// manifest models the release artifact emitted by packaging, with one command release.
func manifest() machinewire.CloudImageManifest {
	runner := commandwire.PackageArtifact{SHA256: strings.Repeat("a", 64), Size: 38710848}
	file := commandwire.PackageArtifact{SHA256: strings.Repeat("c", 64), Size: 2048}
	return machinewire.CloudImageManifest{
		FormatVersion: 1, OS: machinewire.OSLinux, Architecture: machinewire.ArchitectureARM64,
		Rootfs: machinewire.RootfsArchive{SHA256: strings.Repeat("d", 64), Size: 733308752, File: machinewire.RootfsTarZst}, Ubuntu: "26.04",
		Packages:    []machinewire.InstalledPackage{{Name: "adduser", Version: "3.153ubuntu1"}},
		Executables: map[string]commandwire.PackageArtifact{machinewire.RunnerPath: runner, machinewire.InitPath: {SHA256: strings.Repeat("e", 64), Size: 10}, runnerwire.ArtifactsPath + "/" + file.SHA256 + "/demi-file": file},
		Releases:    []commandwire.PackageDescriptor{{ID: "demi.file", Version: "0.1.3", ProtocolVersion: 1, Operations: []string{"file.read"}, Targets: map[string]commandwire.PackageArtifact{"aarch64-unknown-linux-musl": file}}},
		Runner:      runnerwire.RunnerRelease{Release: strings.Repeat("f", 64), Wire: 24, CommandProtocol: 1, Targets: map[string]commandwire.PackageArtifact{"aarch64-unknown-linux-musl": runner}},
		Tools:       []machinewire.StandaloneTool{{Name: "uv", Version: "0.12.13", SHA256: strings.Repeat("9", 64)}},
	}
}

func TestManifestEmbeddedReleases(t *testing.T) {
	data, err := contract.EncodeJSON(manifest())
	if err != nil {
		t.Fatal(err)
	}
	decoded, err := machinewire.DecodeCloudImageManifest(data)
	if err != nil {
		t.Fatal(err)
	}
	if decoded.Architecture != machinewire.ArchitectureARM64 || decoded.Architecture.Target() != "aarch64-unknown-linux-musl" {
		t.Fatal("image target")
	}
	changed := manifest()
	artifact := changed.Executables[machinewire.RunnerPath]
	artifact.Size = 1
	changed.Executables[machinewire.RunnerPath] = artifact
	if !errors.Is(changed.Validate(), machinewire.ErrManifestRunner) {
		t.Fatal("runner mismatch accepted")
	}
	changed = manifest()
	changed.Architecture = machinewire.ArchitectureAMD64
	if !errors.Is(changed.Validate(), machinewire.ErrManifestRunner) {
		t.Fatal("wrong target accepted")
	}
	changed = manifest()
	for path := range changed.Executables {
		if strings.HasPrefix(path, "/opt/") {
			delete(changed.Executables, path)
		}
	}
	var missing *machinewire.ManifestError
	if !errors.As(changed.Validate(), &missing) || missing.Release != "demi.file" {
		t.Fatal("missing release not identified")
	}
	for _, mutation := range []struct{ old, new string }{
		{`"formatVersion":1`, `"formatVersion":2`}, {`"os":"linux"`, `"os":"darwin"`}, {`"ubuntu":"26.04"`, `"ubuntu":""`}, {`"formatVersion":1`, `"extra":true,"formatVersion":1`}, {`rootfs.tar.zst`, `rootfs.tar`},
	} {
		if _, err := machinewire.DecodeCloudImageManifest(bytes.Replace(data, []byte(mutation.old), []byte(mutation.new), 1)); err == nil {
			t.Errorf("accepted %s", mutation.new)
		}
	}
	for _, path := range []string{"/etc/passwd", "/usr/", "/usr/../etc/shadow", "/opt/x\ny", "usr/bin/x"} {
		changed := manifest()
		changed.Executables[path] = commandwire.PackageArtifact{SHA256: strings.Repeat("e", 64), Size: 10}
		if err := changed.Validate(); err == nil {
			t.Errorf("accepted path %q", path)
		}
	}
}

func TestLineLimitAndEscaping(t *testing.T) {
	request := []byte(`{"id":"1","op":"reconcile","params":{}}`)
	atLimit := append(bytes.Repeat([]byte{' '}, machinewire.MaxLineBytes-len(request)), request...)
	if _, err := machinewire.DecodeRequest(atLimit); err != nil {
		t.Fatal(err)
	}
	if _, err := machinewire.DecodeRequest(append(atLimit, ' ')); err == nil {
		t.Fatal("accepted oversized request")
	}
	response := []byte(`{"type":"death","deviceId":"d"}`)
	atLimit = append(bytes.Repeat([]byte{' '}, machinewire.MaxLineBytes-len(response)), response...)
	if _, err := machinewire.DecodeResponse(atLimit); err != nil {
		t.Fatal(err)
	}
	if _, err := machinewire.DecodeResponse(append(atLimit, ' ')); err == nil {
		t.Fatal("accepted oversized response")
	}
	special := "<>&\u2028\u2029"
	data, err := machinewire.EncodeLine(&machinewire.ErrorResponse{ID: "1", Message: special})
	if err != nil || !bytes.Contains(data, []byte(special)) {
		t.Fatalf("escaping: %s %v", data, err)
	}
	for _, bad := range [][]byte{[]byte(`{"type":"death","deviceId":"a","deviceId":"b"}`), []byte("{\"type\":\"death\",\"deviceId\":\"\xff\"}")} {
		if _, err := machinewire.DecodeResponse(bad); err == nil {
			t.Fatal("accepted malformed JSON")
		}
	}
}

// Retain the opaque result's exact integer range until its operation decodes it.
func TestResultIntegerPrecision(t *testing.T) {
	response, err := machinewire.DecodeResponse([]byte(`{"type":"ok","id":"1","result":{"generation":"g","baseVersion":"b","resetId":null,"systemBytes":18446744073709551615,"homeBytes":1}}`))
	if err != nil {
		t.Fatal(err)
	}
	ok, success := response.(*machinewire.OK)
	if !success {
		t.Fatal("expected OK")
	}
	state, err := (machinewire.ImageStateParams{}).DecodeOutput(ok.Result)
	if err != nil || state.SystemBytes != ^uint64(0) {
		t.Fatalf("integer precision: %v %v", state, err)
	}
}
