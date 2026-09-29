package sandbox_test

import (
	"encoding/json/v2"
	"errors"
	"io/fs"
	"net/netip"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"syscall"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/go/machines/internal/config"
	"github.com/wspl/demi/go/machines/internal/sandbox"
	"github.com/wspl/demi/go/machines/internal/tools"
	"github.com/wspl/demi/go/runnerproto"
)

func TestARecordNamesOneBootAndItsSlot(t *testing.T) {
	record, err := sandbox.DecodeRecord([]byte(`{"id":"demi-0f6c3d4e-8a9b-4c1d-9e2f-3a4b5c6d7e8f","slot":3}`))
	if err != nil || record.Slot != 3 {
		t.Fatalf("a valid record: %+v, %v", record, err)
	}
	encoded, err := sandbox.EncodeRecord(record)
	if err != nil || string(encoded) != `{"id":"demi-0f6c3d4e-8a9b-4c1d-9e2f-3a4b5c6d7e8f","slot":3}` {
		t.Errorf("encoded %s, %v", encoded, err)
	}
	for _, invalid := range []string{
		`{"id":"0f6c3d4e","slot":3}`,
		`{"id":"demi-../x","slot":3}`,
		`{"id":"demi-a","slot":-1}`,
		`{"id":"demi-a","slot":65536}`,
		`{"id":"demi-a","slot":3,"token":"x"}`,
	} {
		if _, err := sandbox.DecodeRecord([]byte(invalid)); err == nil {
			t.Errorf("%s decoded", invalid)
		}
	}
	if !strings.HasPrefix(string(sandbox.NewSandboxID()), "demi-") {
		t.Error("a new id does not start with demi-")
	}
}

func TestCredentialModesDoNotDependOnTheUmask(t *testing.T) {
	// The service runs with umask 077; this process has its own now.
	previous := syscall.Umask(0o077)
	defer syscall.Umask(previous)
	directory := sandbox.NewRuntimeDirectory(t.TempDir(), "demi-test")
	if err := directory.Create(); err != nil {
		t.Fatal(err)
	}
	boot, err := runnerproto.DecodeManagedBoot([]byte(`{"backendUrl":"https://backend.example.com","deviceToken":"tok"}`))
	if err != nil {
		t.Fatal(err)
	}
	// Giving the record to the sandbox's user needs root, and comes last.
	written := directory.WriteCredentials(boot, []netip.Addr{netip.MustParseAddr("1.1.1.1")})
	if written != nil && !errors.Is(written, fs.ErrPermission) {
		t.Fatal(written)
	}
	mode := func(path string) (fs.FileMode, *syscall.Stat_t) {
		info, err := os.Stat(path)
		if err != nil {
			t.Fatal(err)
		}
		return info.Mode().Perm(), info.Sys().(*syscall.Stat_t)
	}
	if got, stat := mode(directory.Boot()); got != 0o400 || written == nil && stat.Uid != sandbox.UserID {
		t.Errorf("boot record: mode %o, owner %d", got, stat.Uid)
	}
	if got, _ := mode(directory.Resolver()); got != 0o444 {
		t.Errorf("resolver: mode %o", got)
	}
	if got, _ := mode(directory.Hosts()); got != 0o444 {
		t.Errorf("hosts: mode %o", got)
	}
	if resolver, err := os.ReadFile(directory.Resolver()); err != nil || string(resolver) != "nameserver 1.1.1.1\n" {
		t.Errorf("resolver %q, %v", resolver, err)
	}
	record, err := os.ReadFile(directory.Boot())
	if err != nil {
		t.Fatal(err)
	}
	if decoded, err := runnerproto.DecodeManagedBoot(record); err != nil || decoded.DeviceToken != "tok" {
		t.Errorf("the boot record: %s, %v", record, err)
	}
}

func TestRemovalNeverDeletesWhatAMountPointStillHolds(t *testing.T) {
	directory := sandbox.NewRuntimeDirectory(t.TempDir(), "demi-test")
	if err := directory.Create(); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(directory.Config(), []byte("{}"), 0o644); err != nil {
		t.Fatal(err)
	}
	// A file inside a mount point stands for a mount that survived.
	project := filepath.Join(directory.Home(), "project")
	if err := os.WriteFile(project, []byte("work"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := directory.Remove(); err == nil {
		t.Fatal("removing a directory with a live mount succeeded")
	}
	if got, err := os.ReadFile(project); err != nil || string(got) != "work" {
		t.Fatalf("the mount's contents: %q, %v", got, err)
	}
	if err := os.Remove(project); err != nil {
		t.Fatal(err)
	}
	if err := directory.Remove(); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(directory.Root()); !errors.Is(err, fs.ErrNotExist) {
		t.Errorf("the directory stays: %v", err)
	}
	if err := directory.Remove(); err != nil {
		t.Errorf("removing what is gone: %v", err)
	}
}

func TestTheConfigurationIsTheShippedProfile(t *testing.T) {
	directory := sandbox.NewRuntimeDirectory("/run/demi-machines", "demi-00000000-0000-4000-8000-000000000000")
	spec := sandbox.Spec(sandbox.Boot{
		Directory: directory,
		Namespace: "demi-3",
		Cgroup: &sandbox.CgroupLimits{
			Name:   "demi-00000000-0000-4000-8000-000000000000",
			Limits: config.Limits{CPUs: 2, MemoryMiB: 2048},
		},
	})
	fixture, err := os.ReadFile("../../../../crates/machines/tests/fixtures/oci-config.json")
	if err != nil {
		t.Fatal(err)
	}
	// The file the runtime reads is the fixture's: compared as JSON documents, so
	// a member the file leaves out, such as an empty capability set, differs.
	written, err := sandbox.Config(spec)
	if err != nil {
		t.Fatal(err)
	}
	var got, want any
	if err := json.Unmarshal(written, &got); err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(fixture, &want); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("the configuration differs from the fixture:\n got %s\nwant %s", written, fixture)
	}
	// With the limits off the configuration has neither a cgroup path nor
	// resources.
	unlimited := sandbox.Spec(sandbox.Boot{Directory: directory, Namespace: "demi-3"})
	if unlimited.Linux.CgroupsPath != "" || unlimited.Linux.Resources != nil {
		t.Errorf("a sandbox without limits has a cgroup: %+v", unlimited.Linux)
	}
}

func TestThePinnedRuntimeReleaseIsTheRustManagers(t *testing.T) {
	shipped, err := os.ReadFile("../../../../crates/machines/runtime/release.json")
	if err != nil {
		t.Fatal(err)
	}
	var release sandbox.RuntimeRelease
	if err := json.Unmarshal(shipped, &release, json.RejectUnknownMembers(true)); err != nil {
		t.Fatal(err)
	}
	if release != sandbox.PinnedRelease() {
		t.Errorf("the embedded copy differs from crates/machines/runtime/release.json")
	}
}

func TestTheVersionMustMatchExactly(t *testing.T) {
	version := sandbox.PinnedRelease().Version()
	if !sandbox.ReportsVersion("runsc version "+version+"\nspec: 1.1.0\n", version) {
		t.Error("the version line was refused")
	}
	for _, output := range []string{
		"runsc version " + version + ".1\n",
		"runsc version release-20260914.0-demi.1\n",
		"spec: 1.1.0\nrunsc version " + version + "\n",
	} {
		// On arm64 the pinned version is the patched build's.
		if output == "runsc version "+version+"\n" {
			continue
		}
		if sandbox.ReportsVersion(output, version) {
			t.Errorf("%q was accepted", output)
		}
	}
}

func TestAListingNamesEachContainersStatus(t *testing.T) {
	id := sandbox.SandboxID("demi-a")
	for _, listing := range []string{"null", "[]"} {
		if _, known, err := sandbox.StatusIn(listing, id); known || err != nil {
			t.Errorf("%s: known %v, %v", listing, known, err)
		}
	}
	listing := `[{"id":"demi-b","pid":7,"status":"running"},{"id":"demi-a","pid":9,"status":"paused","bundle":"/x"}]`
	if status, known, err := sandbox.StatusIn(listing, id); status != sandbox.Paused || !known || err != nil {
		t.Errorf("%s: %s, %v, %v", listing, status, known, err)
	}
	if _, _, err := sandbox.StatusIn(`[{"id":"demi-a","status":"exploded"}]`, id); err == nil {
		t.Error("an unknown status was accepted")
	}
}

func TestEveryCommandNamesTheStateRootAndTheProfile(t *testing.T) {
	runsc := sandbox.NewRunsc(&tools.Tools{}, "/run/demi-machines", true)
	want := []string{
		"--root=/run/demi-machines/runsc",
		"--platform=systrap",
		"--network=sandbox",
		"--overlay2=none",
		"--file-access=shared",
		"--file-access-mounts=shared",
		"--allow-suid=true",
		"--directfs=true",
		"pause",
		"demi-a",
	}
	if got := runsc.Args("pause", "demi-a"); !reflect.DeepEqual(got, want) {
		t.Errorf("%v", got)
	}
	// Without cgroups the profile says so: runsc would give each sandbox one.
	without := sandbox.NewRunsc(&tools.Tools{}, "/run/demi-machines", false).Args("pause", "demi-a")
	if got := without[len(without)-3]; got != "--ignore-cgroups" {
		t.Errorf("%v", without)
	}
}

func TestPreparingCgroupsNamesEveryMissingControllerAndChangesNothing(t *testing.T) {
	root := t.TempDir()
	cgroups := sandbox.NewCgroups(root)
	err := cgroups.Prepare()
	var missing *sandbox.MissingControllersError
	if !errors.As(err, &missing) || strings.Join(missing.Names, ",") != "cpu,memory,pids" {
		t.Fatalf("no hierarchy: %v", err)
	}
	if err := os.WriteFile(filepath.Join(root, "cgroup.controllers"), []byte("cpu io pids\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	err = cgroups.Prepare()
	if !errors.As(err, &missing) || strings.Join(missing.Names, ",") != "memory" {
		t.Fatalf("no memory controller: %v", err)
	}
	if !strings.Contains(err.Error(), "missing: memory. DEMI_MANAGED_LIMITS=off runs Clouds without limits") {
		t.Errorf("%v", err)
	}
	if entries, _ := os.ReadDir(root); len(entries) != 1 {
		t.Errorf("a refused start changed the root: %v", entries)
	}
	if err := os.WriteFile(filepath.Join(root, "cgroup.controllers"), []byte("cpu memory pids\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := cgroups.Prepare(); err != nil {
		t.Fatal(err)
	}
	for _, path := range []string{"cgroup.subtree_control", "demi-cloud/cgroup.subtree_control"} {
		if got, err := os.ReadFile(filepath.Join(root, path)); err != nil || string(got) != "+cpu +memory +pids" {
			t.Errorf("%s: %q, %v", path, got, err)
		}
	}
}

func TestFencingKillsWaitsForTheCgroupToEmptyAndRemovesIt(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		root := t.TempDir()
		cgroups := sandbox.NewCgroups(root)
		id := sandbox.SandboxID("demi-a")
		// A sandbox with no cgroup needs no fence.
		if err := cgroups.Fence(t.Context(), id); err != nil {
			t.Fatal(err)
		}
		cgroup := filepath.Join(root, "demi-cloud", string(id))
		if err := os.MkdirAll(cgroup, 0o755); err != nil {
			t.Fatal(err)
		}
		events := filepath.Join(cgroup, "cgroup.events")
		if err := os.WriteFile(events, []byte("populated 1\nfrozen 0\n"), 0o644); err != nil {
			t.Fatal(err)
		}
		fenced := make(chan error)
		go func() { fenced <- cgroups.Fence(t.Context(), id) }()
		// The kill was written; the writers take a second to end.
		time.Sleep(time.Second)
		synctest.Wait()
		if kill, err := os.ReadFile(filepath.Join(cgroup, "cgroup.kill")); err != nil || string(kill) != "1" {
			t.Fatalf("cgroup.kill: %q, %v", kill, err)
		}
		select {
		case err := <-fenced:
			t.Fatalf("the fence ended while writers ran: %v", err)
		default:
		}
		if err := os.WriteFile(events, []byte("populated 0\nfrozen 0\n"), 0o644); err != nil {
			t.Fatal(err)
		}
		// A real cgroup is removed by rmdir whatever its control files; here the
		// directory holds the files the test made, so the removal, the fence's last
		// step, reports it.
		if err := <-fenced; !errors.Is(err, syscall.ENOTEMPTY) {
			t.Fatalf("the fence ended with %v, not at its removal", err)
		}
	})
}

func TestWritersThatOutliveTheKillFailTheFenceAfterFiveSeconds(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		root := t.TempDir()
		cgroups := sandbox.NewCgroups(root)
		id := sandbox.SandboxID("demi-a")
		cgroup := filepath.Join(root, "demi-cloud", string(id))
		if err := os.MkdirAll(cgroup, 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(cgroup, "cgroup.events"), []byte("populated 1\n"), 0o644); err != nil {
			t.Fatal(err)
		}
		start := time.Now()
		err := cgroups.Fence(t.Context(), id)
		if !errors.Is(err, sandbox.ErrWriters) {
			t.Fatalf("%v", err)
		}
		if waited := time.Since(start); waited != 5*time.Second {
			t.Errorf("the fence gave up after %v", waited)
		}
	})
}
