package hostremotetest

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"os"
	"sync"
	"testing"

	"github.com/wspl/demi/go/backendtest"
	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/fixture"
	"github.com/wspl/demi/go/hostremote"
	"github.com/wspl/demi/go/runnerproto"
	"github.com/wspl/demi/go/shell"
	"github.com/wspl/demi/go/shell/shelltest"
)

const TestDeviceID = "test-device"

type CommandPolicy struct {
	Commands  *shell.CommandSet
	mu        sync.Mutex
	storage   map[string]*shelltest.MemoryStorage
	sequences map[string]uint64
}

func NewCommandPolicy(commands *shell.CommandSet) *CommandPolicy {
	if commands == nil {
		commands = &shell.CommandSet{}
	}
	return &CommandPolicy{Commands: commands, storage: map[string]*shelltest.MemoryStorage{}, sequences: map[string]uint64{}}
}
func (*CommandPolicy) AdmitCall(hostremote.JobOrigin) error { return nil }
func (p *CommandPolicy) Dispatch(ctx context.Context, origin hostremote.JobOrigin, invocation shell.RPCInvocation, port shell.RPCPort) (uint8, error) {
	return p.Commands.Dispatch(ctx, invocation, port)
}
func (p *CommandPolicy) StorageFor(node string) *shelltest.MemoryStorage {
	p.mu.Lock()
	defer p.mu.Unlock()
	value := p.storage[node]
	if value == nil {
		value = &shelltest.MemoryStorage{}
		p.storage[node] = value
	}
	return value
}
func (p *CommandPolicy) Storage(ctx context.Context, origin hostremote.JobOrigin, op shell.StorageOp) (shell.StorageReply, error) {
	if origin.Caller == nil {
		return nil, &shell.PortError{Kind: shell.PortStorage, Message: "the job has no command storage"}
	}
	if ctx.Err() != nil {
		return nil, &shell.PortError{Kind: shell.PortEnded, Message: "the call was stopped"}
	}
	return p.StorageFor(origin.Caller.Node.String()).Apply(op), nil
}
func (*CommandPolicy) GrowVolume(context.Context, runnerproto.VolumeName, uint64) error {
	return errors.New("volume growth is not available")
}
func (p *CommandPolicy) ReserveNumbers(ctx context.Context, conversation string, sequence commandservice.Sequence, count uint32) (uint64, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	if err := ctx.Err(); err != nil {
		return 0, err
	}
	key := conversation + ":" + string(sequence)
	first := p.sequences[key]
	if first == 0 {
		first = 1
	}
	p.sequences[key] = first + uint64(count)
	return first, nil
}

type NativeFixture struct {
	Descriptor commandservice.PackageDescriptor
	path       string
}

func LoadNativeFixture(t testing.TB) *NativeFixture {
	t.Helper()
	return NativePackage(t, "demicodes.runner-test", Program(t, "demi-native-fixture"), fixture.Operations())
}
func NativePackage(t testing.TB, id, path string, operations []string) *NativeFixture {
	t.Helper()
	bytes, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	digest := sha256.Sum256(bytes)
	return &NativeFixture{Descriptor: commandservice.PackageDescriptor{ID: id, Version: "test", ProtocolVersion: 1, Operations: operations, Targets: map[commandservice.TargetTriple]commandservice.PackageArtifact{backendtest.HostTarget(t): {SHA256: hex.EncodeToString(digest[:]), Size: uint64(len(bytes))}}}, path: path}
}
func (n *NativeFixture) Resolve(ctx context.Context, artifact commandservice.PackageArtifact, target string) (commandservice.ArtifactLocation, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if artifact != n.Descriptor.Targets[commandservice.TargetTriple(target)] {
		return nil, errors.New("the artifact is not the fixture's")
	}
	return commandservice.ArtifactPath{Path: n.path}, nil
}
