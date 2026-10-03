package plugin

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/host"
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
			declare.Leaf[declare.NativeOperation]{
				Name:    "list",
				Summary: "List",
				Kind:    &declare.RPC[declare.NativeOperation]{},
			},
			handler,
		),
	)
	p, err := NewCommandPlugin(PlacementDemi, []host.Declared{tree})
	if err != nil {
		t.Fatal(err)
	}
	inv := host.RPCInvocation{Path: []string{"file", "list"}, Args: json.RawMessage(`{}`)}
	if err := p.Check(inv); err != nil {
		t.Fatal(err)
	}
	reply, err := p.Call(t.Context(), &RequestCommand{Invocation: inv}, Port{})
	if err != nil {
		t.Fatal(err)
	}
	if reply.(*ReplyExit).Code != 7 || strings.Join(inv.Path, " ") != "file list" {
		t.Fatalf("reply %v, path %v", reply, inv.Path)
	}
	commands := p.ManifestCommands()
	if len(commands) != 1 || declare.Name(commands[0].Tree.Node) != "file" || commands[0].Placement != PlacementDemi {
		t.Fatalf("commands = %#v", commands)
	}
	_, err = p.Call(t.Context(), &RequestContext{}, Port{})
	var failed *ErrorFailed
	if !errors.As(err, &failed) || failed.Message != "the plugin declares no context source" {
		t.Fatalf("context = %v", err)
	}
	inv.Path = []string{"file", "missing"}
	var usage *ErrorUsage
	if err := p.Check(inv); !errors.As(err, &usage) {
		t.Fatalf("unknown command = %v", err)
	}
}

// TestDirectoryIdentity protects path ordering, mode-sensitive identity and
// the directory/path rules at the Host installation boundary.
func TestDirectoryIdentity(t *testing.T) {
	blob := core.BlobRefOf([]byte("contents"))
	d := HostDirectory{
		Name:  "test",
		Files: []DirectoryFile{{Path: "b", Blob: blob}, {Path: "a", Blob: blob, Executable: true}},
	}
	reverse := HostDirectory{Name: "test", Files: []DirectoryFile{d.Files[1], d.Files[0]}}
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
	for _, dirs := range [][]HostDirectory{
		{{Name: "UPPER"}},
		{{Name: "same"}, {Name: "same"}},
		{{Name: "valid", Files: []DirectoryFile{{Path: "../escape"}}}},
		{{Name: "valid", Files: []DirectoryFile{{Path: "a"}, {Path: "a"}}}},
	} {
		if err := CheckDirectories(dirs); err == nil {
			t.Fatalf("invalid directory set accepted: %#v", dirs)
		}
	}
	if err := CheckDirectories([]HostDirectory{d}); err != nil {
		t.Fatal(err)
	}
}

// TestErrorClassification protects the refusal and cancellation distinctions
// used by the web and rpc adapters.
func TestErrorClassification(t *testing.T) {
	ended := RequestError(&host.PortError{Kind: host.PortEnded, Message: "cancelled"})
	if _, ok := ended.(*ErrorEnded); !ok {
		t.Fatalf("ended = %T", ended)
	}
	var port *host.PortError
	if err := RPCError(ended); !errors.As(err, &port) || port.Kind != host.PortEnded {
		t.Fatalf("rpc ended = %v", err)
	}
	conflict := &PortRefusalConflict{}
	converted := RequestError(conflict)
	if !errors.Is(converted, conflict) {
		t.Fatal("refusal identity lost")
	}
	for _, r := range []Request{&RequestCommand{}, &RequestContext{}, &RequestPageState{}, &RequestPageCall{}} {
		_, err := (NoRequests{}).Call(t.Context(), r, Port{})
		var failed *ErrorFailed
		if !errors.As(err, &failed) {
			t.Fatalf("identity plugin accepted %T", r)
		}
	}
}

// TestMessageObjectBoundaries protects object parameters that opaque JSON
// alone would otherwise allow to be scalar, and validates plugin identifiers.
func TestMessageObjectBoundaries(t *testing.T) {
	for _, id := range []string{"", "execution", "UPPER", strings.Repeat("a", 33), "é"} {
		_, err := ParseID(id)
		want := `"` + id + `" is not a plugin id: 1 to 32 lowercase letters, digits and hyphens, not "execution"`
		if err == nil || err.Error() != want {
			t.Fatalf("id %q = %v", id, err)
		}
	}
	if _, err := ParseID("a-1"); err != nil {
		t.Fatal(err)
	}
	if _, err := DecodeRequest(
		[]byte(`{"type":"page_call","user":"aaaaaaaaaaaaaaaaaaaaaaaaaa","method":"x","params":[]}`),
	); err == nil {
		t.Fatal("array page params accepted")
	}
	if _, err := DecodePortMessage(
		[]byte(`{"type":"package_call","operation":{"package":"x","operation":"y"},"args":false,"kind":"looks"}`),
	); err == nil {
		t.Fatal("boolean package args accepted")
	}
}

type answerTransport struct {
	answer PortAnswer
	err    error
}

func (a answerTransport) Request(context.Context, PortMessage) (PortAnswer, error) {
	return a.answer, a.err
}

// TestPortFailureRouting protects the distinction between a refusal, a broken
// transport and an answer to the wrong operation, including the nested rpc port.
func TestPortFailureRouting(t *testing.T) {
	p := NewPort(answerTransport{answer: &PortAnswerDone{}})
	_, _, err := p.Value(t.Context(), "x")
	var wrong *host.PortError
	if !errors.As(err, &wrong) || wrong.Kind != host.UnexpectedReply || wrong.Asked != "read_value" ||
		wrong.Answered != "done" {
		t.Fatalf("wrong answer = %v", err)
	}
	refusal := &PortRefusalNotRunning{}
	p = NewPort(answerTransport{answer: &PortAnswerRefused{Refusal: refusal}})
	_, _, err = p.Value(t.Context(), "x")
	if !errors.Is(err, refusal) {
		t.Fatalf("refusal = %v", err)
	}
	err = p.RPC().Stdout(t.Context(), []byte("x"))
	if !errors.As(err, &wrong) || wrong.Answered != "refused" || wrong.Asked != "rpc" {
		t.Fatalf("rpc refusal = %v", err)
	}
	failure := errors.New("disconnected")
	p = NewPort(answerTransport{err: failure})
	if err := p.RemoveExpose(t.Context(), ""); !errors.Is(err, failure) {
		t.Fatalf("transport = %v", err)
	}
	p = NewPort(answerTransport{answer: &PortAnswerRPC{Response: &host.PortWritten{}}})
	if err := p.RPC().Stdout(t.Context(), []byte("x")); err != nil {
		t.Fatal(err)
	}
}
