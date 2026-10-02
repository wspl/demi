package runner

import (
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/runnerwire"
)

// These scenarios cost about a second each: they start a real runner and jobs.
func TestFilesystemRequestsAndKillRemainAvailableDuringJob(t *testing.T) {
	f := newRunner(t, nil, "")
	f.online()
	f.job("live", "printf ready; sleep 60")
	if _, ok := f.frame().(*runnerwire.JobOutput); !ok {
		t.Fatal("job did not start")
	}
	f.send(&runnerwire.FSExists{ID: "file", Path: "."})
	reply, ok := f.frame().(*runnerwire.FSOK)
	if !ok || reply.ID != "file" {
		t.Fatal("filesystem request blocked by job")
	}
	exists, ok := reply.Result.(*runnerwire.FSExistsResult)
	if !ok || !exists.Value {
		t.Fatalf("exists result: %#v", reply.Result)
	}
	f.send(&runnerwire.JobKill{JobID: "live", Signal: new(runnerwire.SignalKill)})
	if exit, ok := f.frame().(*runnerwire.JobExit); !ok || exit.JobID != "live" {
		t.Fatal("kill did not end job")
	}
}
func TestAStartTheRunnerCannotBeginReportsSpawnError(t *testing.T) {
	f := newRunner(t, nil, "")
	f.online()
	f.job("twin", "printf ready; sleep 60")
	if _, ok := f.frame().(*runnerwire.JobOutput); !ok {
		t.Fatal("job did not start")
	}
	f.job("twin", "sleep 60")
	exit, ok := f.frame().(*runnerwire.JobExit)
	if !ok || exit.ExitCode != nil || exit.Signal != nil || exit.SpawnError == nil || exit.SpawnError.Kind != runnerwire.SpawnErrorKindOther || exit.SpawnError.Detail == nil || *exit.SpawnError.Detail != "duplicate live task id" {
		t.Fatalf("duplicate start: %#v", exit)
	}
}
func TestJobEnvironmentCombinesDeviceRequestAndOwnedValues(t *testing.T) {
	f := newRunner(t, map[string]string{"DEVICE": "device", "OVERRIDE": "old"}, "")
	f.online()
	if err := os.WriteFile(filepath.Join(f.home, ".profile"), []byte("profile_home=\"$HOME\"\n"), 0600); err != nil {
		t.Fatal(err)
	}
	f.send(&runnerwire.JobStart{JobID: "env", Context: runnerCommandContext(), Script: `printf '%s:%s:%s:%s:%s' "$DEVICE" "$OVERRIDE" "$DEMI_HOME" "$HOME" "$profile_home"`, CWD: f.home, Env: map[string]string{"OVERRIDE": "new", "DEMI_HOME": "untrusted"}})
	out, stderr, exit := f.jobOutput("env")
	requireJobSuccess(t, exit, stderr)
	want := "device:new:" + f.state + ":" + f.home + ":" + f.home
	if out != want {
		t.Fatalf("environment %q, want %q", out, want)
	}
}
func TestRawSpawnInheritsEnvironmentOnlyWhenRequested(t *testing.T) {
	f := newRunner(t, map[string]string{"DEVICE": "device", "OVERRIDE": "device"}, "")
	f.online()
	for _, test := range []struct {
		name    string
		inherit *bool
		env     *map[string]*string
		want    string
	}{
		{"default", nil, nil, "device:device"},
		{"replace", nil, &map[string]*string{"OVERRIDE": new("caller")}, "unset:caller"},
		{"extend", new(true), &map[string]*string{"OVERRIDE": new("caller")}, "device:caller"},
		{"remove", new(true), &map[string]*string{"DEVICE": nil}, "unset:device"},
	} {
		command := "/usr/bin/env"
		var args *[]string
		if runtime.GOOS == "windows" {
			command = `C:\Windows\System32\cmd.exe`
			args = new([]string{"/c", "set"})
		}
		f.send(&runnerwire.Spawn{SpawnID: test.name, Command: command, Args: args, Env: test.env, InheritEnv: test.inherit, KillProcessGroup: new(true)})
		var out strings.Builder
		for {
			message := f.frame()
			if exit, ok := message.(*runnerwire.SpawnExit); ok {
				if exit.ExitCode == nil || *exit.ExitCode != 0 {
					t.Fatalf("spawn: %+v", exit)
				}
				break
			}
			if chunk, ok := message.(*runnerwire.SpawnOutput); ok && chunk.Stream == runnerwire.Stdout {
				out.Write(chunk.Bytes)
			}
		}
		values := map[string]string{"DEVICE": "unset", "OVERRIDE": "unset"}
		for _, line := range strings.Split(out.String(), "\n") {
			if name, value, ok := strings.Cut(strings.TrimSuffix(line, "\r"), "="); ok {
				values[name] = value
			}
		}
		if got := values["DEVICE"] + ":" + values["OVERRIDE"]; got != test.want {
			t.Fatalf("%s: %q, want %q", test.name, got, test.want)
		}
	}
}

// directoriesBecome observes cleanup using protocol round trips rather than timed sleeps.
func (f *runnerFixture) directoriesBecome(count int) {
	f.t.Helper()
	for {
		entries, err := os.ReadDir(filepath.Join(f.state, "jobs"))
		if err != nil && !os.IsNotExist(err) {
			f.t.Fatal(err)
		}
		found := 0
		for _, entry := range entries {
			if entry.IsDir() {
				found++
			}
		}
		if found == count {
			return
		}
		f.send(&runnerwire.Ping{})
		if _, ok := f.frame().(*runnerwire.Pong); !ok {
			f.t.Fatal("unexpected work during cleanup")
		}
	}
}
func TestJobDirectoriesFollowReleaseConnectionAndNextStart(t *testing.T) {
	f := newRunner(t, nil, "")
	f.stop()
	left := filepath.Join(f.state, "jobs", "job-left", "output")
	if err := os.MkdirAll(left, 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(left, "head"), []byte("abandoned output"), 0600); err != nil {
		t.Fatal(err)
	}
	f.start()
	f.online()
	f.directoriesBecome(0)
	f.job("one", "printf done")
	_, stderr, exit := f.jobOutput("one")
	requireJobSuccess(t, exit, stderr)
	f.job("live", "printf ready; sleep 60")
	if _, ok := f.frame().(*runnerwire.JobOutput); !ok {
		t.Fatal("live job did not start")
	}
	f.directoriesBecome(2)
	f.send(&runnerwire.JobRelease{JobID: "one"})
	f.send(&runnerwire.JobRelease{JobID: "live"})
	f.directoriesBecome(1)
	f.stop()
	entries, err := os.ReadDir(filepath.Join(f.state, "jobs"))
	if err != nil {
		t.Fatal(err)
	}
	for _, entry := range entries {
		if entry.IsDir() {
			t.Fatalf("directory survived disconnect: %s", entry.Name())
		}
	}
}
