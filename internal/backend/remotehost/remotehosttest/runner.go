package remotehosttest

//revive:disable:unused-parameter
// API checkpoint: keep parameter names for callers until the bodies are implemented.

import (
	"context"
	"os/exec"
	"testing"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerwire"
)

// FixtureOptions supplies a runner's environment, callback commands and optional wire tap.
type FixtureOptions struct {
	Env      map[string]string
	Commands *host.CommandSet
	Tap      chan<- runnerwire.Outbound
}

// RunnerFixture owns a real runner connected to a backend end of its own.
// Construction registers cleanup for the process, server, connections and pipes.
type RunnerFixture struct{ _ byte }

// StartRunnerFixture starts the backend and runner and waits until it is online.
func StartRunnerFixture(ctx context.Context, t testing.TB, options FixtureOptions) (*RunnerFixture, error) {
	panic("not written: b-remotehost")
}

// Home returns the runner's private home path.
func (f *RunnerFixture) Home() string { panic("not written: b-remotehost") }

// JobRoot returns the runner's job directory root.
func (f *RunnerFixture) JobRoot() string { panic("not written: b-remotehost") }

// JobDirectories lists the runner's retained job directories.
func (f *RunnerFixture) JobDirectories(ctx context.Context) ([]string, error) {
	panic("not written: b-remotehost")
}

// Host returns the device Host rooted at the runner's home.
func (f *RunnerFixture) Host() *remotehost.Host { panic("not written: b-remotehost") }

// HostAt returns the device Host with the supplied working directory.
func (f *RunnerFixture) HostAt(cwd string) *remotehost.Host { panic("not written: b-remotehost") }

// Pipes returns the fixture's pipe broker.
func (f *RunnerFixture) Pipes() *remotehost.Pipes { panic("not written: b-remotehost") }

// Policy returns the fixture's callback policy and storage.
func (f *RunnerFixture) Policy() *CommandPolicy { panic("not written: b-remotehost") }

// Link waits for the runner's live connection.
func (f *RunnerFixture) Link(ctx context.Context) (*remotehost.Link, error) {
	panic("not written: b-remotehost")
}

// Offline waits for the runner to disconnect.
func (f *RunnerFixture) Offline(ctx context.Context) error { panic("not written: b-remotehost") }

// Command prepares a CLI command using the fixture's runner state.
// The caller owns starting and waiting for it.
func (f *RunnerFixture) Command(ctx context.Context) *exec.Cmd { panic("not written: b-remotehost") }

// Log returns the runner's captured output.
func (f *RunnerFixture) Log() string { panic("not written: b-remotehost") }

// Stop stops the runner and backend and joins all owned work.
func (f *RunnerFixture) Stop(ctx context.Context) error { panic("not written: b-remotehost") }
