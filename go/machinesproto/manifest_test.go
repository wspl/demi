package machinesproto_test

import (
	"encoding/json/v2"
	"errors"
	"maps"
	"strings"
	"testing"

	"github.com/wspl/demi/go/machinesproto"
)

// manifest is a small manifest in the shape the packaging writes, for an arm64
// image embedding one command package.
func manifest() map[string]any {
	// Each use gets its own map, so a test can change one.
	runner := func() map[string]any { return map[string]any{"sha256": strings.Repeat("a", 64), "size": 38710848} }
	builtin := func() map[string]any { return map[string]any{"sha256": strings.Repeat("c", 64), "size": 2048} }
	return map[string]any{
		"formatVersion": 1,
		"os":            "linux",
		"architecture":  "arm64",
		"rootfs":        map[string]any{"sha256": strings.Repeat("d", 64), "size": 733308752, "file": "rootfs.tar.zst"},
		"ubuntu":        "26.04",
		"packages":      []any{map[string]any{"name": "adduser", "version": "3.153ubuntu1"}},
		"executables": map[string]any{
			"/usr/bin/demi-runner": runner(),
			"/usr/bin/tini":        map[string]any{"sha256": strings.Repeat("e", 64), "size": 10},
			"/opt/demi/artifacts/" + strings.Repeat("c", 64) + "/demi-commands": builtin(),
		},
		"releases": []any{map[string]any{
			"id":              "demi.builtin",
			"version":         "0.1.3",
			"protocolVersion": 1,
			"operations":      []any{"file.read"},
			"targets":         map[string]any{"aarch64-unknown-linux-musl": builtin()},
		}},
		"runner": map[string]any{
			"release":         strings.Repeat("f", 64),
			"wire":            24,
			"commandProtocol": 1,
			"targets":         map[string]any{"aarch64-unknown-linux-musl": runner()},
		},
		"tools": []any{map[string]any{"name": "uv", "version": "0.12.13", "sha256": strings.Repeat("9", 64)}},
	}
}

func decode(t *testing.T, value map[string]any) (*machinesproto.CloudImageManifest, error) {
	t.Helper()
	data, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	return machinesproto.DecodeManifest(data)
}

func TestAnImageManifestNamesItsEmbeddedRunnerAndPackages(t *testing.T) {
	decoded, err := decode(t, manifest())
	if err != nil {
		t.Fatalf("a valid manifest: %v", err)
	}
	if decoded.Architecture != machinesproto.Arm64 || decoded.Architecture.Target() != "aarch64-unknown-linux-musl" {
		t.Errorf("architecture %q, target %q", decoded.Architecture, decoded.Architecture.Target())
	}

	changed := manifest()
	changed["executables"].(map[string]any)["/usr/bin/demi-runner"].(map[string]any)["size"] = 1
	if _, err := decode(t, changed); !errors.Is(err, machinesproto.ErrRunner) {
		t.Errorf("a runner that is not the embedded one: %v", err)
	}
	changed = manifest()
	changed["architecture"] = "amd64"
	if _, err := decode(t, changed); !errors.Is(err, machinesproto.ErrRunner) {
		t.Errorf("a runner of another architecture: %v", err)
	}
	changed = manifest()
	executables := changed["executables"].(map[string]any)
	for path := range maps.Clone(executables) {
		if strings.HasPrefix(path, "/opt/") {
			delete(executables, path)
		}
	}
	_, err = decode(t, changed)
	var refused *machinesproto.ManifestError
	if !errors.As(err, &refused) || refused.Release != "demi.builtin" {
		t.Errorf("a package that is not embedded: %v", err)
	}

	for field, value := range map[string]any{
		"formatVersion": 2,
		"os":            "darwin",
		"ubuntu":        "",
		"extra":         true,
	} {
		changed = manifest()
		changed[field] = value
		if _, err := decode(t, changed); err == nil {
			t.Errorf("%s: %v was accepted", field, value)
		}
	}
	for _, path := range []string{"/etc/passwd", "/usr/", "/usr/../etc/shadow", "/opt/x\ny", "usr/bin/x"} {
		changed = manifest()
		changed["executables"].(map[string]any)[path] = map[string]any{"sha256": strings.Repeat("e", 64), "size": 10}
		if _, err := decode(t, changed); err == nil {
			t.Errorf("%q was accepted", path)
		}
	}
	// The values of another package's types are checked by that package's rules.
	changed = manifest()
	changed["runner"].(map[string]any)["targets"].(map[string]any)["sparc-unknown-linux-gnu"] = map[string]any{"sha256": strings.Repeat("a", 64), "size": 1}
	if _, err := decode(t, changed); err == nil || !strings.Contains(err.Error(), "runner.targets") {
		t.Errorf("a runner target that is no platform: %v", err)
	}
	changed = manifest()
	changed["executables"].(map[string]any)["/usr/bin/tini"].(map[string]any)["sha256"] = "XYZ"
	if _, err := decode(t, changed); err == nil || !strings.Contains(err.Error(), "executables") {
		t.Errorf("an executable with a bad digest: %v", err)
	}
	changed = manifest()
	changed["releases"].([]any)[0].(map[string]any)["id"] = "Not An Id"
	if _, err := decode(t, changed); err == nil || !strings.Contains(err.Error(), "releases[0].id") {
		t.Errorf("a release with a bad id: %v", err)
	}
	changed = manifest()
	changed["rootfs"].(map[string]any)["file"] = "rootfs.tar"
	if _, err := decode(t, changed); err == nil {
		t.Error("another archive file was accepted")
	}
	for _, member := range []string{"packages", "executables", "releases", "runner", "tools", "rootfs"} {
		changed = manifest()
		delete(changed, member)
		if _, err := decode(t, changed); err == nil {
			t.Errorf("a manifest without %s was accepted", member)
		}
	}
}
