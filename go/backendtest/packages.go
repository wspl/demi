package backendtest

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json/v2"
	"io"
	"os"
	"path/filepath"
	"runtime"
	"sync"
	"testing"

	"github.com/wspl/demi/go/builtinproto"
	"github.com/wspl/demi/go/commandservice"
)

// A Package is a native command package the workspace built: the release the
// backend's local store serves its runners, from which every runner, a paired
// device's or the Cloud's, downloads the program (native-runtime.md § Backend
// deployment configuration).
type Package struct {
	// ID is the package's id in the native catalog.
	ID string
	// Program is the executable's name in the programs directory.
	Program string
	// Operations are the operations the package serves.
	Operations []string
}

// The packages a scenario loads, one at a time as the Rust suite does.
var (
	// BuiltinPackage is demi.builtin, from demi-commands.
	BuiltinPackage = Package{ID: builtinproto.Package, Program: "demi-commands", Operations: builtinproto.Operations()}
	// ClaudePackage is demi.claude, from demi-claude.
	ClaudePackage = Package{ID: "demi.claude", Program: "demi-claude", Operations: []string{"claude.ensure", "claude.status"}}
	// FixturePackage is the runner's native fixture: echo answers what the page
	// sends, where reports its context and directory, retain makes the
	// resident service hold the conversation, held answers what the service
	// holds, stall_release holds the conversation so that its release never
	// ends by itself, and stalled ends once such a release waits.
	FixturePackage = Package{
		ID:      "demicodes.runner-test",
		Program: "demi-native-fixture",
		Operations: []string{
			"where", "echo", "first", "spin", "result", "retain", "stall_release", "held", "crash", "stalled",
			"proceed", "number",
		},
	}
)

// FixtureStreams are the user streams the native fixture serves: the names of
// the operations a page opens through the conversation's stream route.
var FixtureStreams = []string{"echo", "where", "retain", "held", "stall_release", "stalled"}

// releases publishes each package once per test process: reading the whole
// program for its digest takes most of a second for demi-commands.
var releases struct {
	mu      sync.Mutex
	root    string
	built   map[string]string
	digests map[string]artifact
}

// An artifact is a program's SHA-256 and size.
type artifact struct {
	sha256 string
	size   uint64
}

// Main runs a package's tests and removes what they published. A package's
// TestMain calls it.
func Main(m *testing.M) int {
	code := m.Run()
	releases.mu.Lock()
	defer releases.mu.Unlock()
	if releases.root != "" {
		// What is left is a temporary directory of ours; the test process ends.
		_ = os.RemoveAll(releases.root)
	}
	return code
}

// releaseDirectory returns the release directory of pkg, writing it the first
// time: descriptor.json and, for this machine's target, a link to the program.
func releaseDirectory(t testing.TB, pkg Package) string {
	t.Helper()
	releases.mu.Lock()
	defer releases.mu.Unlock()
	if directory, ok := releases.built[pkg.Program]; ok {
		return directory
	}
	if releases.root == "" {
		root, err := os.MkdirTemp("", "demi-releases-")
		if err != nil {
			t.Fatal(err)
		}
		releases.root = root
		releases.built = map[string]string{}
		releases.digests = map[string]artifact{}
	}
	program := Program(t, pkg.Program)
	found, err := digestOf(program)
	if err != nil {
		t.Fatal(err)
	}
	releases.digests[pkg.Program] = found
	digest, size := found.sha256, found.size
	target := HostTarget(t)
	descriptor := commandservice.PackageDescriptor{
		ID:              pkg.ID,
		Version:         "test",
		ProtocolVersion: commandservice.Version,
		Operations:      pkg.Operations,
		Targets: map[commandservice.TargetTriple]commandservice.PackageArtifact{
			target: {SHA256: digest, Size: size},
		},
	}
	encoded, err := commandservice.Encode(descriptor)
	if err != nil {
		t.Fatalf("the descriptor of %s: %v", pkg.ID, err)
	}
	directory := filepath.Join(releases.root, pkg.Program)
	executables := filepath.Join(directory, string(target))
	if err := os.MkdirAll(executables, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(program, filepath.Join(executables, pkg.Program)); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(directory, "descriptor.json"), encoded, 0o644); err != nil {
		t.Fatal(err)
	}
	releases.built[pkg.Program] = directory
	return directory
}

// writeNativeConfig writes the native configuration that names the releases of
// packages, in dir, and answers its path.
func writeNativeConfig(t testing.TB, dir string, packages []Package) string {
	t.Helper()
	type release struct {
		Directory  string `json:"directory"`
		Executable string `json:"executable"`
	}
	config := struct {
		Releases []release         `json:"releases"`
		Store    map[string]string `json:"store"`
	}{Releases: []release{}, Store: map[string]string{"provider": "local"}}
	for _, pkg := range packages {
		config.Releases = append(config.Releases, release{Directory: releaseDirectory(t, pkg), Executable: pkg.Program})
	}
	encoded, err := json.Marshal(config)
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(dir, "native.json")
	if err := os.WriteFile(path, encoded, 0o644); err != nil {
		t.Fatal(err)
	}
	return path
}

// digestOf returns the SHA-256 and the size of the file at path.
func digestOf(path string) (artifact, error) {
	file, err := os.Open(path)
	if err != nil {
		return artifact{}, err
	}
	defer file.Close()
	sum := sha256.New()
	size, err := io.Copy(sum, file)
	if err != nil {
		return artifact{}, err
	}
	return artifact{sha256: hex.EncodeToString(sum.Sum(nil)), size: uint64(size)}, nil
}

// Artifact returns the SHA-256 and the size of the package's program for this
// machine's target, as its descriptor declares them, and the program's path.
func (p Package) Artifact(t testing.TB) (digest string, size uint64, program string) {
	t.Helper()
	releaseDirectory(t, p)
	releases.mu.Lock()
	defer releases.mu.Unlock()
	found := releases.digests[p.Program]
	return found.sha256, found.size, Program(t, p.Program)
}

// HostTarget is the target triple of the machine the suite runs on, which is
// the one the programs are built for.
func HostTarget(t testing.TB) commandservice.TargetTriple {
	t.Helper()
	switch runtime.GOOS + "/" + runtime.GOARCH {
	case "linux/amd64":
		return "x86_64-unknown-linux-musl"
	case "linux/arm64":
		return "aarch64-unknown-linux-musl"
	case "darwin/amd64":
		return "x86_64-apple-darwin"
	case "darwin/arm64":
		return "aarch64-apple-darwin"
	}
	t.Skipf("no native package target for %s/%s", runtime.GOOS, runtime.GOARCH)
	return ""
}
