//go:build unix

package tabs

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
)

// directoryBases isolates both sweep passes from other tests and Host browsers.
func directoryBases(t *testing.T) DirectoryBases {
	t.Helper()
	root := t.TempDir()
	bases := DirectoryBases{Runtime: filepath.Join(root, "runtime"), Profiles: filepath.Join(root, "profiles")}
	for _, path := range []string{bases.Runtime, bases.Profiles} {
		if err := os.Mkdir(path, 0700); err != nil {
			t.Fatal(err)
		}
	}
	return bases
}
func directoryFixture(t *testing.T, bases DirectoryBases, id string) (string, string) {
	t.Helper()
	runtime := filepath.Join(bases.Runtime, runtimePrefix+id)
	profile := filepath.Join(bases.Profiles, profilePrefix+id)
	for _, path := range []string{runtime, profile} {
		if err := os.Mkdir(path, 0700); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.Symlink(profile, filepath.Join(runtime, "profile")); err != nil {
		t.Fatal(err)
	}
	return runtime, profile
}
func lockFixture(t *testing.T, path string, held bool) {
	t.Helper()
	file, err := os.OpenFile(filepath.Join(path, profileLock), os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		t.Fatal(err)
	}
	if held {
		t.Cleanup(func() {
			if err := file.Close(); err != nil {
				t.Error(err)
			}
		})
		locked, err := lockEnvironment(file)
		if err != nil || !locked {
			t.Fatal(locked, err)
		}
	} else if err := file.Close(); err != nil {
		t.Fatal(err)
	}
}
func requireExists(t *testing.T, want bool, paths ...string) {
	t.Helper()
	for _, path := range paths {
		_, err := os.Lstat(path)
		if want && err != nil || !want && !errors.Is(err, os.ErrNotExist) {
			t.Fatalf("%s: exists want %v: %v", path, want, err)
		}
	}
}

func TestSweepRemovesOnlyEnvironmentsNoServiceHolds(t *testing.T) {
	bases := directoryBases(t)
	orphan, orphanProfile := directoryFixture(t, bases, "orphan")
	lockFixture(t, orphan, false)
	held, heldProfile := directoryFixture(t, bases, "held")
	lockFixture(t, held, true)
	staged, stagedProfile := directoryFixture(t, bases, "being-made")
	other := filepath.Join(bases.Runtime, "not-an-environment")
	if err := os.Mkdir(other, 0700); err != nil {
		t.Fatal(err)
	}
	lockFixture(t, other, false)
	if err := sweepOrphansIn(t.Context(), bases, nil, currentOwner()); err != nil {
		t.Fatal(err)
	}
	requireExists(t, false, orphan, orphanProfile)
	requireExists(t, true, held, heldProfile, staged, stagedProfile, other)
}

func TestSweepRemovesProfileWhoseRuntimeIsGone(t *testing.T) {
	bases := directoryBases(t)
	runtime, left := directoryFixture(t, bases, "emptied")
	if err := os.RemoveAll(runtime); err != nil {
		t.Fatal(err)
	}
	held, heldProfile := directoryFixture(t, bases, "held")
	lockFixture(t, held, true)
	other := filepath.Join(bases.Profiles, "not-a-profile")
	if err := os.Mkdir(other, 0700); err != nil {
		t.Fatal(err)
	}
	if err := sweepOrphansIn(t.Context(), bases, nil, currentOwner()); err != nil {
		t.Fatal(err)
	}
	requireExists(t, false, left)
	requireExists(t, true, heldProfile, other)
}

func TestSweepFollowsOrphanLinkOnlyToOwnedProfile(t *testing.T) {
	bases := directoryBases(t)
	elsewhere := filepath.Join(t.TempDir(), "elsewhere")
	if err := os.Mkdir(elsewhere, 0700); err != nil {
		t.Fatal(err)
	}
	symbolic := filepath.Join(bases.Profiles, profilePrefix+"symbolic")
	if err := os.Symlink(elsewhere, symbolic); err != nil {
		t.Fatal(err)
	}
	paths := []string{elsewhere, symbolic}
	if os.Getuid() == 0 {
		foreign := filepath.Join(bases.Profiles, profilePrefix+"foreign")
		if err := os.Mkdir(foreign, 0700); err != nil {
			t.Fatal(err)
		}
		if err := os.Chown(foreign, 65534, 65534); err != nil {
			t.Fatal(err)
		}
		paths = append(paths, foreign)
	}
	var runtimes []string
	for _, path := range paths {
		runtime, err := os.MkdirTemp(bases.Runtime, runtimePrefix)
		if err != nil {
			t.Fatal(err)
		}
		lockFixture(t, runtime, false)
		if err := os.Symlink(path, filepath.Join(runtime, "profile")); err != nil {
			t.Fatal(err)
		}
		runtimes = append(runtimes, runtime)
	}
	if err := sweepOrphansIn(t.Context(), bases, nil, currentOwner()); err != nil {
		t.Fatal(err)
	}
	requireExists(t, false, runtimes...)
	requireExists(t, true, paths...)
}

func TestSweepLeavesOtherUsersEnvironmentsAlone(t *testing.T) {
	bases := directoryBases(t)
	orphan, profile := directoryFixture(t, bases, "orphan")
	lockFixture(t, orphan, false)
	runtime, left := directoryFixture(t, bases, "emptied")
	if err := os.RemoveAll(runtime); err != nil {
		t.Fatal(err)
	}
	if err := sweepOrphansIn(t.Context(), bases, nil, currentOwner()+1); err != nil {
		t.Fatal(err)
	}
	requireExists(t, true, orphan, profile, left)
	if err := sweepOrphansIn(t.Context(), bases, nil, currentOwner()); err != nil {
		t.Fatal(err)
	}
	requireExists(t, false, orphan, profile, left)
}

// This filesystem race ports the Rust 4,000-creation/three-sweeper scenario.
// It costs several seconds on macOS; no in-memory fake can prove the atomic
// publication of an OS lock to another concurrent directory scanner.
func TestSweepTakesNoEnvironmentBeingMade(t *testing.T) {
	bases := directoryBases(t)
	ctx, cancel := context.WithCancel(t.Context())
	var sweeps sync.WaitGroup
	for range 3 {
		sweeps.Go(func() {
			for ctx.Err() == nil {
				if err := sweepOrphansIn(ctx, bases, nil, currentOwner()); err != nil && !errors.Is(err, context.Canceled) {
					t.Error(err)
					return
				}
			}
		})
	}
	defer func() {
		cancel()
		sweeps.Wait()
	}()
	for range 4000 {
		directories, err := createDirectories(bases)
		if err != nil {
			t.Fatal(err)
		}
		profile, err := os.Stat(directories.profile)
		if err != nil || !profile.IsDir() {
			t.Fatalf("profile is not a directory: %v %v", profile, err)
		}
		info, err := os.Stat(filepath.Join(directories.runtime, profileLock))
		if err != nil || !info.Mode().IsRegular() {
			t.Fatalf("environment lock is not a file: %v %v", info, err)
		}
		if err := directories.remove(t.Context(), nil); err != nil {
			t.Fatal(err)
		}
	}
}

func TestFailedProcessRetirementRetainsDirectoriesAndCause(t *testing.T) {
	directories, err := createDirectories(directoryBases(t))
	if err != nil {
		t.Fatal(err)
	}
	failure := &cdp.BrowserError{Kind: cdp.KindTimeout}
	err = directories.remove(t.Context(), failure)
	var retained *cdp.BrowserError
	if !errors.As(err, &retained) || retained.Kind != cdp.KindProfileRetained || retained.Path != directories.profile || !errors.Is(err, failure) {
		t.Fatalf("retention lost cause: %v", err)
	}
	for _, path := range []string{directories.runtime, directories.profile} {
		info, err := os.Stat(path)
		if err != nil || !info.IsDir() {
			t.Fatalf("retained path is not a directory: %s %v", path, err)
		}
	}
}
