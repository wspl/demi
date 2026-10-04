package remotehost_test

import (
	"context"
	"reflect"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/cmddecl"
	"github.com/wspl/demi/internal/cmdproto"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerproto"
)

// artifactScript records resolver admission and optionally waits for its job's cancellation.
type artifactScript struct {
	calls   atomic.Int32
	wait    bool
	started chan context.Context
	done    chan struct{}
}

func (s *artifactScript) Resolve(
	ctx context.Context,
	_ cmdproto.PackageArtifact,
	_ string,
) (cmdproto.ArtifactLocation, error) {
	s.calls.Add(1)
	if s.started != nil {
		s.started <- ctx
	}
	if s.wait {
		<-ctx.Done()
		close(s.done)
		return nil, ctx.Err()
	}
	return &cmdproto.ArtifactURL{URL: "https://artifacts.example.test/exact"}, nil
}

// catalogFixture pins an executable and one companion archive to a native command.
func catalogFixture(t *testing.T, r remotehost.ArtifactResolver) (*remotehost.CommandSelection, string) {
	t.Helper()
	target, err := cmdproto.HostTarget()
	requirePipe(t, err)
	descriptor := cmdproto.PackageDescriptor{
		ID:              "demicodes.fixture",
		Version:         "1.0.0",
		ProtocolVersion: 1,
		Operations:      []string{"file.read"},
		Targets: map[string]cmdproto.PackageArtifact{
			string(target): {SHA256: strings.Repeat("a", 64), Size: 1},
		},
		Resources: map[string]cmdproto.PackageResource{
			"chrome": {
				Title: "Chrome for Testing 153.0.8010.36",
				Targets: map[string]cmdproto.ResourceArtifact{
					string(target): {SHA256: strings.Repeat("b", 64), Size: 2, Entry: "chrome-linux64/chrome"},
				},
			},
		},
	}
	catalog, err := remotehost.NewCommandCatalog([]cmdproto.PackageDescriptor{descriptor}, r)
	requirePipe(t, err)
	commands := (&host.CommandSet{})
	requirePipe(
		t,
		commands.Register(
			host.Leaf(
				cmddecl.Leaf[cmddecl.NativeOperation]{
					Name:    "native",
					Summary: "Native",
					Kind: &cmddecl.Native[cmddecl.NativeOperation]{
						Binding: cmddecl.NativeOperation{Package: descriptor.ID, Operation: "file.read"},
					},
				},
				nil,
			),
		),
	)
	selection, err := catalog.Select(commands)
	requirePipe(t, err)
	return selection, string(target)
}

// rpcCommands constructs a declared callback used for manifest identity checks.
func rpcCommands(t *testing.T, name, summary string) *host.CommandSet {
	t.Helper()
	commands := (&host.CommandSet{})
	requirePipe(
		t,
		commands.Register(
			host.Leaf(
				cmddecl.Leaf[cmddecl.NativeOperation]{
					Name:    name,
					Summary: summary,
					Kind:    &cmddecl.RPC[cmddecl.NativeOperation]{},
				},
				host.RPCHandlerFunc(
					func(context.Context, host.RPCInvocation, host.RPCPort) (uint8, error) {
						return 0, nil
					},
				),
			),
		),
	)
	return commands
}

func TestJobsShareSelectedManifestOnlyWithinOneConnection(t *testing.T) {
	d, l, h := linkDevice(t)
	catalog, err := remotehost.NewCommandCatalog(nil, &artifactScript{})
	requirePipe(t, err)
	first, err := catalog.Select((&host.CommandSet{}))
	requirePipe(t, err)
	second, err := catalog.Select(rpcCommands(t, "example", "Example command"))
	requirePipe(t, err)
	for i, selection := range []*remotehost.CommandSelection{first, first, second, first} {
		request := startRequest("true")
		request.Commands = selection
		_, err = h.StartJob(t.Context(), request)
		requirePipe(t, err)
		if i != 1 {
			frame := nextFrame(t, l).(*runnerproto.ManifestMessage)
			manifest, err := runnerproto.DecodeManifest(frame.Manifest)
			requirePipe(t, err)
			if manifest.Hash != selection.Hash() {
				t.Fatal("wrong manifest hash")
			}
		}
		if _, ok := nextFrame(t, l).(*runnerproto.JobStart); !ok {
			t.Fatal("missing job start")
		}
	}
	barrier(t, l)
	_, err = l.Close(t.Context())
	requirePipe(t, err)
	l = d.Connect(0)
	request := startRequest("true")
	request.Commands = first
	_, err = h.StartJob(t.Context(), request)
	requirePipe(t, err)
	_ = nextFrame(t, l).(*runnerproto.ManifestMessage)
	_ = nextFrame(t, l).(*runnerproto.JobStart)
	barrier(t, l)
}

func TestArtifactRequestNeedsLiveJobAndItsManifestArtifact(t *testing.T) {
	_, l, h := linkDevice(t)
	resolver := &artifactScript{}
	selection, target := catalogFixture(t, resolver)
	start := startRequest("native")
	start.Commands = selection
	job, err := h.StartJob(t.Context(), start)
	requirePipe(t, err)
	nextFrame(t, l)
	nextFrame(t, l)
	request := func(hash string) {
		sendFrame(
			t,
			l,
			&runnerproto.ArtifactResolve{
				ID:     "request",
				Owner:  &runnerproto.JobArtifactOwner{JobID: job.ID(), ManifestHash: selection.Hash()},
				SHA256: hash,
				Target: target,
			},
		)
	}
	request(strings.Repeat("f", 64))
	answer := nextFrame(t, l).(*runnerproto.ArtifactLocation)
	if answer.Location != nil || answer.Error == nil ||
		*answer.Error != "Artifact does not belong to the live work's packages" ||
		resolver.calls.Load() != 0 {
		t.Fatal(answer)
	}
	for i, hash := range []string{strings.Repeat("a", 64), strings.Repeat("b", 64)} {
		request(hash)
		answer = nextFrame(t, l).(*runnerproto.ArtifactLocation)
		if answer.Location == nil {
			t.Fatal("artifact location missing", answer)
		}
		location, ok := (*answer.Location).(*cmdproto.ArtifactURL)
		if answer.Error != nil || !ok || location.URL != "https://artifacts.example.test/exact" ||
			location.ExpiresAt != nil ||
			resolver.calls.Load() != int32(i+1) {
			t.Fatal(answer)
		}
	}
	sendFrame(
		t,
		l,
		&runnerproto.JobExit{JobID: job.ID(), ExitCode: new(int32(0)), Files: []runnerproto.JobFileChange{}},
	)
	request(strings.Repeat("a", 64))
	answer = nextFrame(t, l).(*runnerproto.ArtifactLocation)
	if answer.Location != nil || answer.Error == nil || *answer.Error != "No matching live job or stream" ||
		resolver.calls.Load() != 2 {
		t.Fatal(answer)
	}
}

func TestEndingJobCancelsPendingArtifactsWithoutAnswer(t *testing.T) {
	_, l, h := linkDevice(t)
	resolver := &artifactScript{wait: true, started: make(chan context.Context, 1), done: make(chan struct{})}
	selection, target := catalogFixture(t, resolver)
	start := startRequest("native")
	start.Commands = selection
	job, err := h.StartJob(t.Context(), start)
	requirePipe(t, err)
	nextFrame(t, l)
	nextFrame(t, l)
	request := &runnerproto.ArtifactResolve{
		ID:     "request",
		Owner:  &runnerproto.JobArtifactOwner{JobID: job.ID(), ManifestHash: selection.Hash()},
		SHA256: strings.Repeat("a", 64),
		Target: target,
	}
	sendFrame(t, l, request)
	observed := <-resolver.started
	sendFrame(t, l, request)
	answer := nextFrame(t, l).(*runnerproto.ArtifactLocation)
	if answer.Location != nil || answer.Error == nil ||
		*answer.Error != "Artifact resolution request limit or duplicate id" ||
		observed.Err() != nil {
		t.Fatal(answer)
	}
	sendFrame(
		t,
		l,
		&runnerproto.JobExit{JobID: job.ID(), ExitCode: new(int32(0)), Files: []runnerproto.JobFileChange{}},
	)
	<-resolver.done
	barrier(t, l)
	if l.Queued() != 0 {
		t.Fatal("stale artifact answer")
	}
}

func TestRunnerInstallsAreConnectionsLastList(t *testing.T) {
	d, l, _ := linkDevice(t)
	installs, changed := l.Link().WatchInstalls()
	if len(installs) != 0 {
		t.Fatal(installs)
	}
	install := runnerproto.Install{
		Package: "demi.browser",
		Name:    "Chrome for Testing",
		Version: "153.0.8010.36",
		Phase:   runnerproto.InstallPhaseDownload,
		Done:    40,
		Total:   196,
	}
	sendFrame(t, l, &runnerproto.Installs{Installs: []runnerproto.Install{install}})
	<-changed
	if !reflect.DeepEqual(l.Link().Installs(), []runnerproto.Install{install}) {
		t.Fatal(l.Link().Installs())
	}
	_, changed = l.Link().WatchInstalls()
	sendFrame(t, l, &runnerproto.Installs{Installs: []runnerproto.Install{}})
	<-changed
	if len(l.Link().Installs()) != 0 {
		t.Fatal("old installs retained")
	}
	_, err := l.Close(t.Context())
	requirePipe(t, err)
	if len(d.Connect(0).Link().Installs()) != 0 {
		t.Fatal("installs crossed connections")
	}
}
