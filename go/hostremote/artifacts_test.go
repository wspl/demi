package hostremote_test

import (
	"context"
	"strings"
	"testing"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandtree"
	"github.com/wspl/demi/go/hostremote"
	"github.com/wspl/demi/go/hostremote/hostremotetest"
	"github.com/wspl/demi/go/runnerproto"
	"github.com/wspl/demi/go/shell"
)

type heldResolver struct {
	entered chan struct{}
	ended   chan struct{}
}

func (r *heldResolver) Resolve(ctx context.Context, _ commandservice.PackageArtifact, _ string) (commandservice.ArtifactLocation, error) {
	r.entered <- struct{}{}
	<-ctx.Done()
	r.ended <- struct{}{}
	return nil, ctx.Err()
}

// Cost: one fake connection, 32 blocked resolutions. Exits and request replies
// provide barriers; no elapsed-time assumption decides admission or cleanup.
func TestArtifactAdmissionPinsPackagesAndCancelsWithTheJob(t *testing.T) {
	device := hostremotetest.NewTestDevice(t, nil)
	link := device.Connect(0)
	resolver := &heldResolver{entered: make(chan struct{}, 32), ended: make(chan struct{}, 32)}
	descriptor := commandservice.PackageDescriptor{ID: "demicodes.fixture", Version: "test", ProtocolVersion: 1, Operations: []string{"echo"}, Targets: map[commandservice.TargetTriple]commandservice.PackageArtifact{"x86_64-unknown-linux-musl": {SHA256: strings.Repeat("a", 64), Size: 1}}}
	catalog, err := hostremote.NewCommandCatalog([]commandservice.PackageDescriptor{descriptor}, resolver)
	if err != nil {
		t.Fatal(err)
	}
	commands := &shell.CommandSet{}
	if err := commands.Register(shell.DeclareLeaf(commandtree.Leaf{Name: "probe", Summary: "probe", Kind: commandtree.KindNative, Binding: &commandtree.Binding{Package: descriptor.ID, Operation: "echo"}}, nil)); err != nil {
		t.Fatal(err)
	}
	selection, err := catalog.Select(commands)
	if err != nil {
		t.Fatal(err)
	}
	request := jobStart()
	request.Commands = selection
	first, err := device.Host("/work", nil).StartJob(t.Context(), request)
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := next(t, link).(runnerproto.InboundManifest); !ok {
		t.Fatal("manifest missing")
	}
	next(t, link)
	second, err := device.Host("/work", nil).StartJob(t.Context(), request)
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := next(t, link).(runnerproto.InboundJobStart); !ok {
		t.Fatal("manifest repeated")
	}
	owner := runnerproto.JobArtifactOwner{JobID: first.ID(), ManifestHash: selection.Hash()}
	bad := runnerproto.OutboundArtifactResolve{ID: "unknown", Owner: runnerproto.JobArtifactOwner{JobID: "gone", ManifestHash: selection.Hash()}, SHA256: strings.Repeat("a", 64), Target: "x86_64-unknown-linux-musl"}
	send(t, link, bad)
	if answer := next(t, link).(runnerproto.InboundArtifactLocation); answer.Error == nil || *answer.Error != "No matching live job or stream" {
		t.Fatal(answer)
	}
	bad.ID = "foreign"
	bad.Owner = owner
	bad.SHA256 = strings.Repeat("b", 64)
	send(t, link, bad)
	if answer := next(t, link).(runnerproto.InboundArtifactLocation); answer.Error == nil || *answer.Error != "Artifact does not belong to the live work's packages" {
		t.Fatal(answer)
	}
	good := bad
	good.SHA256 = strings.Repeat("a", 64)
	for n := range 32 {
		good.ID = string(rune('a' + n))
		send(t, link, good)
		<-resolver.entered
	}
	send(t, link, good)
	if answer := next(t, link).(runnerproto.InboundArtifactLocation); answer.Error == nil || *answer.Error != "Artifact resolution request limit or duplicate id" {
		t.Fatal(answer)
	}
	good.ID = "overflow"
	send(t, link, good)
	if answer := next(t, link).(runnerproto.InboundArtifactLocation); answer.Error == nil {
		t.Fatal("33rd resolution admitted")
	}
	send(t, link, runnerproto.OutboundJobExit{JobID: first.ID(), ExitCode: new(int32(0)), Files: []runnerproto.JobFileChange{}})
	for range 32 {
		<-resolver.ended
	}
	if _, err := first.End(t.Context()); err != nil {
		t.Fatal(err)
	}
	// Another request supplies a routing barrier after job_exit. No cancelled
	// resolution may answer before this explicit refusal or after it.
	good.ID = "after"
	send(t, link, good)
	if answer := next(t, link).(runnerproto.InboundArtifactLocation); answer.ID != "after" || answer.Error == nil || *answer.Error != "No matching live job or stream" {
		t.Fatal(answer)
	}
	send(t, link, runnerproto.OutboundJobExit{JobID: second.ID(), ExitCode: new(int32(0)), Files: []runnerproto.JobFileChange{}})
	if _, err := second.End(t.Context()); err != nil {
		t.Fatal(err)
	}
}
