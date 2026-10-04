package main

import (
	"bytes"
	"errors"
	"fmt"
	"io"
	"maps"
	"os"
	"path/filepath"
	"reflect"
	"slices"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandpackage/file/fileproto"
	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/runnerproto"
	"github.com/wspl/demi/internal/version"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }
func writeFixture(t *testing.T, path string, data []byte) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, data, 0o755); err != nil {
		t.Fatal(err)
	}
}

func readFixture(t *testing.T, path string) []byte {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return data
}

func nativeFixture(t *testing.T, root, program string, targets []string, contents string) {
	t.Helper()
	for _, target := range targets {
		writeFixture(
			t,
			filepath.Join(root, ".cache/native-target", target, "release", executableName(program, target)),
			[]byte(contents+" "+target),
		)
	}
}

func appFixture(t *testing.T) *application {
	t.Helper()
	return &application{Root: t.TempDir(), Out: io.Discard, Err: io.Discard, chromeRelease: browserproto.PinnedRelease}
}

func readRunner(t *testing.T, root string) runnerproto.Release {
	t.Helper()
	release, err := runnerproto.DecodeRelease(readFixture(t, filepath.Join(root, "manifest.json")))
	if err != nil {
		t.Fatal(err)
	}
	return release
}

func requireConflict(t *testing.T, err error, paths ...string) {
	t.Helper()
	const conflict = " is already published with other contents"
	if err == nil || !strings.HasSuffix(err.Error(), conflict) {
		t.Fatalf("wanted immutable publication conflict, got %v", err)
	}
	if len(paths) != 0 && !strings.HasSuffix(err.Error(), paths[0]+conflict) {
		t.Fatalf("conflict %v, want path %s", err, paths[0])
	}
}

func TestNamedTargetsMustBeEachExecutablesAndUnnamedAreAllOfTheirs(t *testing.T) {
	linux := []string{"aarch64-unknown-linux-musl", "x86_64-unknown-linux-musl"}
	for _, test := range []struct {
		programs, named, want []string
		refused               bool
	}{
		{[]string{"demi-runner", "demi-machine-manager"}, []string{commandproto.Targets[0]}, nil, true},
		{[]string{"demi-runner", "demi-machine-manager"}, []string{linux[1], linux[0]}, linux, false},
		{[]string{"demi-machine-manager"}, nil, linux, false},
		{defaultPrograms, nil, commandproto.Targets, false},
	} {
		got, err := selectTargets(test.programs, test.named)
		if test.refused && (err == nil || err.Error() != "demi-machine-manager is not built for aarch64-apple-darwin") {
			t.Fatalf("target refusal: %v", err)
		}
		if (err != nil) != test.refused || !reflect.DeepEqual(got, test.want) {
			t.Fatalf("targets(%v,%v) = %v, %v; want %v", test.programs, test.named, got, err, test.want)
		}
	}
	a := appFixture(t)
	if err := a.run(
		t.Context(),
		[]string{"native", "build", "--package", "demi-machine-manager", "--target", commandproto.Targets[0]},
	); err == nil ||
		err.Error() != "demi-machine-manager is not built for aarch64-apple-darwin" {
		t.Fatal("unsupported build accepted")
	}
	if _, err := os.Stat(filepath.Join(a.Root, ".cache")); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("invalid selection started building: %v", err)
	}
}

func TestEachRunnerReleaseHasItsDirectoryAndManifestNamesLastInPlace(t *testing.T) {
	a := appFixture(t)
	output := filepath.Join(a.Root, "runners")
	args := []string{"native", "package", "--package", "demi-runner", "--output", output}
	nativeFixture(t, a.Root, "demi-runner", commandproto.Targets, "first")
	if err := a.run(t.Context(), args); err != nil {
		t.Fatal(err)
	}
	first := readRunner(t, output)
	if len(first.Targets) != 6 {
		t.Fatal(first)
	}
	if !bytes.Equal(
		readFixture(t, filepath.Join(output, "manifest.json")),
		readFixture(t, filepath.Join(output, first.Release, "manifest.json")),
	) {
		t.Fatal("pointer differs")
	}
	windows := filepath.Join(output, first.Release, "x86_64-pc-windows-msvc/demi-runner.exe")
	if string(readFixture(t, windows)) != "first x86_64-pc-windows-msvc" {
		t.Fatal("wrong Windows artifact")
	}
	if err := a.run(t.Context(), args); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(readRunner(t, output), first) {
		t.Fatal("repeat changed identity")
	}
	nativeFixture(t, a.Root, "demi-runner", commandproto.Targets, "second")
	if err := a.run(t.Context(), args); err != nil {
		t.Fatal(err)
	}
	second := readRunner(t, output)
	if second.Release == first.Release {
		t.Fatal("build changed without new identity")
	}
	requireReleaseEntries(t, output, first.Release, second.Release, "manifest.json")
	writeFixture(t, filepath.Join(output, first.Release, commandproto.Targets[2], "demi-runner"), []byte("corrupt"))
	nativeFixture(t, a.Root, "demi-runner", commandproto.Targets, "first")
	requireConflict(
		t,
		a.run(t.Context(), args),
		filepath.Join(output, first.Release, commandproto.Targets[2], "demi-runner"),
	)
	if !reflect.DeepEqual(readRunner(t, output), second) {
		t.Fatal("failed publication moved pointer")
	}
	requireReleaseEntries(t, output, first.Release, second.Release, "manifest.json")
}

func TestBackendOrManagerReleaseRecordsVersionAndIsImmutable(t *testing.T) {
	for _, program := range []string{"demi-backend", "demi-machine-manager"} {
		t.Run(program, func(t *testing.T) {
			a := appFixture(t)
			targets, err := selectTargets([]string{program}, nil)
			if err != nil {
				t.Fatal(err)
			}
			nativeFixture(t, a.Root, program, targets, "server")
			output := filepath.Join(a.Root, program)
			args := []string{"native", "package", "--package", program, "--output", output}
			if err := a.run(t.Context(), args); err != nil {
				t.Fatal(err)
			}
			release, err := decodeExecutableRelease(readFixture(t, filepath.Join(output, "release.json")))
			if err != nil {
				t.Fatal(err)
			}
			if release.Executable != program || release.Version != version.Release ||
				len(release.Targets) != len(targets) {
				t.Fatal(release)
			}
			for _, target := range targets {
				got, err := measureExecutable(
					t.Context(),
					filepath.Join(a.Root, ".cache/native-target", target, "release", executableName(program, target)),
				)
				if err != nil {
					t.Fatal(err)
				}
				if !bytes.Equal(
					readFixture(t, filepath.Join(output, target, executableName(program, target))),
					[]byte("server "+target),
				) {
					t.Fatal("published executable differs from build")
				}
				if got != release.Targets[target] {
					t.Fatal("release hash differs")
				}
			}
			requireReleaseEntries(t, output, append(slices.Clone(targets), "release.json")...)
			nativeFixture(t, a.Root, program, targets, "rebuilt")
			requireConflict(t, a.run(t.Context(), args))
		})
	}
}

func TestDevelopmentReleaseCarriesNamedTargetsAndProgramsOperations(t *testing.T) {
	a := appFixture(t)
	targets := []string{commandproto.Targets[0], commandproto.Targets[3]}
	nativeFixture(t, a.Root, "demi-file", targets, "file")
	output := filepath.Join(a.Root, "file")
	args := []string{"native", "package", "--package", "demi-file", "--output", output}
	missingBuild := a.run(t.Context(), args)
	// The message names the first missing build and how to make it.
	source := filepath.Join(a.Root, ".cache/native-target", commandproto.Targets[1], "release", "demi-file")
	want := fmt.Sprintf(
		"no build of demi-file for %s at %s: run go run ./tools/release native build first: ",
		commandproto.Targets[1],
		source,
	)
	if !errors.Is(missingBuild, os.ErrNotExist) || !strings.HasPrefix(missingBuild.Error(), want) {
		t.Fatalf("missing build: %v, want %q and the file's error", missingBuild, want)
	}
	if _, err := os.Stat(output); !errors.Is(err, os.ErrNotExist) {
		t.Fatal("published incomplete release")
	}
	for _, target := range targets {
		args = append(args, "--target", target)
	}
	if err := a.run(t.Context(), args); err != nil {
		t.Fatal(err)
	}
	descriptor, err := commandproto.DecodePackageDescriptor(readFixture(t, filepath.Join(output, "descriptor.json")))
	if err != nil {
		t.Fatal(err)
	}
	if descriptor.ID != fileproto.Package || descriptor.Version != version.Release ||
		!reflect.DeepEqual(descriptor.Operations, fileproto.Operations()) ||
		len(descriptor.Resources) != 0 ||
		!slices.Equal(slices.Sorted(maps.Keys(descriptor.Targets)), targets) {
		t.Fatal(descriptor)
	}
	requireReleaseEntries(t, output, append(slices.Clone(targets), "descriptor.json")...)
	nativeFixture(t, a.Root, "demi-runner", targets, "runner")
	runnerOutput := filepath.Join(a.Root, "runners")
	if err := a.packageNative(
		t.Context(),
		packageOptions{Package: "demi-runner", Output: runnerOutput, Targets: targets},
	); err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(slices.Sorted(maps.Keys(readRunner(t, runnerOutput).Targets)), targets) {
		t.Fatal("wrong development runner targets")
	}
}

func TestCommandPackageCarriesResourcesForItsTargets(t *testing.T) {
	// Accepted R5 removal: Chrome is the only resource a command package carries.
	a := appFixture(t)
	target := commandproto.Targets[3]
	program := filepath.Join(a.Root, "browser")
	writeFixture(t, program, []byte("browser"))
	cache := filepath.Join(a.Root, ".cache/resources")
	archive := []byte("chrome archive")
	temp := filepath.Join(cache, "written")
	writeFixture(t, temp, archive)
	digest, err := measureExecutable(t.Context(), temp)
	if err != nil {
		t.Fatal(err)
	}
	cached := filepath.Join(cache, digest.SHA256)
	writeFixture(t, cached, archive)
	chrome := browserproto.BrowserRelease{Version: "153.0.8010.36"}
	for _, target := range []string{target, commandproto.Targets[0]} {
		chrome.Platforms = append(
			chrome.Platforms,
			browserproto.ReleasePlatform{
				Target:     target,
				URL:        "https://example.test/chrome.zip",
				Size:       digest.Size,
				SHA256:     digest.SHA256,
				Executable: "chrome-linux64/chrome",
			},
		)
	}
	options := packageOptions{Package: "demi-browser", Output: filepath.Join(a.Root, "release")}
	sources := map[string]string{target: program}
	if err := a.publishNative(t.Context(), options, sources, chrome); err != nil {
		t.Fatal(err)
	}
	descriptor, err := commandproto.DecodePackageDescriptor(
		readFixture(t, filepath.Join(options.Output, "descriptor.json")),
	)
	if err != nil {
		t.Fatal(err)
	}
	if len(descriptor.Resources) != 1 {
		t.Fatalf("resources: %v", descriptor.Resources)
	}
	resource := descriptor.Resources["chrome"]
	if resource.Title != chrome.Title() || len(resource.Targets) != 1 ||
		resource.Targets[target].SHA256 != digest.SHA256 ||
		resource.Targets[target].Size != digest.Size ||
		resource.Targets[target].Entry != "chrome-linux64/chrome" {
		t.Fatal(resource)
	}
	requireReleaseEntries(t, options.Output, "descriptor.json", "resources", target)
	if !bytes.Equal(archive, readFixture(t, filepath.Join(options.Output, "resources", digest.SHA256))) {
		t.Fatal("wrong resource bytes")
	}
	if err := a.publishNative(t.Context(), options, sources, chrome); err != nil {
		t.Fatal("cached repeat:", err)
	}
	writeFixture(t, cached, []byte("corrupt"))
	options.Output = filepath.Join(a.Root, "bad-release")
	if err := a.publishNative(t.Context(), options, sources, chrome); err == nil {
		t.Fatal("corrupt cache accepted")
	}
	if _, err := os.Stat(options.Output); !errors.Is(err, os.ErrNotExist) {
		t.Fatal("corrupt release published")
	}
}

// requireReleaseEntries checks the complete published directory, including absence of stages.
func requireReleaseEntries(t *testing.T, path string, want ...string) {
	t.Helper()
	entries, err := os.ReadDir(path)
	if err != nil {
		t.Fatal(err)
	}
	names := make([]string, len(entries))
	for i, entry := range entries {
		names[i] = entry.Name()
	}
	slices.Sort(want)
	if !slices.Equal(names, want) {
		t.Fatalf("%s entries: %v, want %v", path, names, want)
	}
}
