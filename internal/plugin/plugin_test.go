package plugin_test

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/cmddecl"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/types"
)

// TestCommandPlacement protects tree-relative dispatch and checks without
// mutating the invocation owned by the plugin caller.
func TestCommandPlacement(t *testing.T) {
	handler := host.RPCHandlerFunc(func(_ context.Context, inv host.RPCInvocation, _ host.RPCPort) (uint8, error) {
		if strings.Join(inv.Path, " ") != "demi file list" {
			t.Fatalf("handler path = %v", inv.Path)
		}
		return 7, nil
	})
	tree := host.Group(
		"file",
		"Files",
		host.Leaf(
			cmddecl.Leaf[cmddecl.NativeOperation]{
				Name:    "list",
				Summary: "List",
				Kind:    &cmddecl.RPC[cmddecl.NativeOperation]{},
			},
			handler,
		),
	)
	p, err := plugin.NewCommandPlugin(plugin.PlacementDemi, []host.Declared{tree})
	if err != nil {
		t.Fatal(err)
	}
	inv := host.RPCInvocation{Path: []string{"file", "list"}, Args: json.RawMessage(`{}`)}
	if err := p.Check(inv); err != nil {
		t.Fatal(err)
	}
	reply, err := p.Call(t.Context(), &plugin.RequestCommand{Invocation: inv}, plugin.Port{})
	if err != nil {
		t.Fatal(err)
	}
	if reply.(*plugin.ReplyExit).Code != 7 || strings.Join(inv.Path, " ") != "file list" {
		t.Fatalf("reply %v, path %v", reply, inv.Path)
	}
	commands := p.ManifestCommands()
	if len(commands) != 1 || cmddecl.Name(commands[0].Tree.Node) != "file" ||
		commands[0].Placement != plugin.PlacementDemi {
		t.Fatalf("commands = %#v", commands)
	}
	_, err = p.Call(t.Context(), &plugin.RequestContext{}, plugin.Port{})
	var failed *plugin.ErrorFailed
	if !errors.As(err, &failed) || failed.Message != "the plugin declares no context source" {
		t.Fatalf("context = %v", err)
	}
	inv.Path = []string{"file", "missing"}
	var usage *plugin.ErrorUsage
	if err := p.Check(inv); !errors.As(err, &usage) {
		t.Fatalf("unknown command = %v", err)
	}
}

// TestDirectoryIdentity protects path ordering, mode-sensitive identity and
// the directory/path rules at the Host installation boundary.
func TestDirectoryIdentity(t *testing.T) {
	blob := types.BlobRefOf([]byte("contents"))
	d := plugin.HostDirectory{
		Name:  "test",
		Files: []plugin.DirectoryFile{{Path: "b", Blob: blob}, {Path: "a", Blob: blob, Executable: true}},
	}
	reverse := plugin.HostDirectory{Name: "test", Files: []plugin.DirectoryFile{d.Files[1], d.Files[0]}}
	if d.Digest() != reverse.Digest() {
		t.Fatal("digest depends on declaration order")
	}
	if d.Digest() != "71cc963ab0685cbb63894547e5e4d859a1ce87838567d5b30e641b6c04c14b57" {
		t.Fatalf("digest = %s", d.Digest())
	}
	reverse.Files[0].Executable = false
	if d.Digest() == reverse.Digest() {
		t.Fatal("mode is missing from digest")
	}
	for _, dirs := range [][]plugin.HostDirectory{
		{{Name: "UPPER"}},
		{{Name: "same"}, {Name: "same"}},
		{{Name: "valid", Files: []plugin.DirectoryFile{{Path: "../escape"}}}},
		{{Name: "valid", Files: []plugin.DirectoryFile{{Path: "a"}, {Path: "a"}}}},
	} {
		if err := plugin.CheckDirectories(dirs); err == nil {
			t.Fatalf("invalid directory set accepted: %#v", dirs)
		}
	}
	if err := plugin.CheckDirectories([]plugin.HostDirectory{d}); err != nil {
		t.Fatal(err)
	}
}

// TestErrorClassification protects the refusal and cancellation distinctions
// used by the web and rpc adapters.
func TestErrorClassification(t *testing.T) {
	ended := plugin.RequestError(&host.PortError{Kind: host.PortEnded, Message: "cancelled"})
	if _, ok := ended.(*plugin.ErrorEnded); !ok {
		t.Fatalf("ended = %T", ended)
	}
	var port *host.PortError
	if err := plugin.RPCError(ended); !errors.As(err, &port) || port.Kind != host.PortEnded {
		t.Fatalf("rpc ended = %v", err)
	}
	conflict := &plugin.PortRefusalConflict{}
	converted := plugin.RequestError(conflict)
	if !errors.Is(converted, conflict) {
		t.Fatal("refusal identity lost")
	}
	for _, r := range []plugin.Request{
		&plugin.RequestCommand{}, &plugin.RequestContext{}, &plugin.RequestPageState{},
		&plugin.RequestPageCall{}, &plugin.RequestPanelTab{}, &plugin.RequestTopic{},
	} {
		_, err := (plugin.NoRequests{}).Call(t.Context(), r, plugin.Port{})
		var failed *plugin.ErrorFailed
		if !errors.As(err, &failed) {
			t.Fatalf("identity plugin accepted %T", r)
		}
	}
}

// TestMessageObjectBoundaries protects object parameters that opaque JSON
// alone would otherwise allow to be scalar, and validates plugin identifiers.
func TestMessageObjectBoundaries(t *testing.T) {
	for _, id := range []string{"", "execution", "UPPER", strings.Repeat("a", 33), "é"} {
		_, err := plugin.ParseID(id)
		want := `"` + id + `" is not a plugin id: 1 to 32 lowercase letters, digits and hyphens, not "execution"`
		if err == nil || err.Error() != want {
			t.Fatalf("id %q = %v", id, err)
		}
	}
	if _, err := plugin.ParseID("a-1"); err != nil {
		t.Fatal(err)
	}
	if _, err := plugin.DecodeRequest(
		[]byte(`{"type":"page_call","user":"aaaaaaaaaaaaaaaaaaaaaaaaaa","method":"x","params":[]}`),
	); err == nil {
		t.Fatal("array page params accepted")
	}
	if _, err := plugin.DecodePortMessage(
		[]byte(`{"type":"package_call","operation":{"package":"x","operation":"y"},"args":false,"kind":"looks"}`),
	); err == nil {
		t.Fatal("boolean package args accepted")
	}
}

type answerTransport struct {
	answer plugin.PortAnswer
	err    error
}

func (a answerTransport) Request(context.Context, plugin.PortMessage) (plugin.PortAnswer, error) {
	return a.answer, a.err
}

// TestPortFailureRouting protects the distinction between a refusal, a broken
// transport and an answer to the wrong operation, including the nested rpc port.
func TestPortFailureRouting(t *testing.T) {
	p := plugin.NewPort(answerTransport{answer: &plugin.PortAnswerDone{}})
	_, _, err := p.Value(t.Context(), "x")
	var wrong *host.PortError
	if !errors.As(err, &wrong) || wrong.Kind != host.UnexpectedReply || wrong.Asked != "read_value" ||
		wrong.Answered != "done" {
		t.Fatalf("wrong answer = %v", err)
	}
	refusal := &plugin.PortRefusalNotRunning{}
	p = plugin.NewPort(answerTransport{answer: &plugin.PortAnswerRefused{Refusal: refusal}})
	_, _, err = p.Value(t.Context(), "x")
	if !errors.Is(err, refusal) {
		t.Fatalf("refusal = %v", err)
	}
	err = p.RPC().Stdout(t.Context(), []byte("x"))
	if !errors.As(err, &wrong) || wrong.Answered != "refused" || wrong.Asked != "rpc" {
		t.Fatalf("rpc refusal = %v", err)
	}
	failure := errors.New("disconnected")
	p = plugin.NewPort(answerTransport{err: failure})
	if err := p.RemoveExpose(t.Context(), ""); !errors.Is(err, failure) {
		t.Fatalf("transport = %v", err)
	}
	p = plugin.NewPort(answerTransport{answer: &plugin.PortAnswerRPC{Response: &host.PortWritten{}}})
	if err := p.RPC().Stdout(t.Context(), []byte("x")); err != nil {
		t.Fatal(err)
	}
}
