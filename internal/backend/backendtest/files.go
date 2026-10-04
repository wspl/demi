package backendtest

import (
	"context"
	"errors"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"strings"

	"github.com/fsnotify/fsnotify"
	"github.com/wspl/demi/internal/commanddecl"
	"github.com/wspl/demi/internal/plugin"
)

// CommandProbe prints the invoking user and the plugin-relative command path.
// Its manifest is configurable for assembled registry scenarios.
type CommandProbe struct {
	// Declaration contains the configurable command probe manifest.
	Declaration plugin.Manifest
}

// Manifest returns the scenario's declarations.
func (p *CommandProbe) Manifest() plugin.Manifest { return p.Declaration }

// Instance creates the stateless command printer.
func (*CommandProbe) Instance() plugin.Plugin { return commandPrinter{} }

type commandPrinter struct{}

// Call handles the fixture plugin request.
func (commandPrinter) Call(ctx context.Context, request plugin.Request, port plugin.Port) (plugin.Reply, error) {
	call, ok := request.(*plugin.RequestCommand)
	if !ok {
		return nil, &plugin.ErrorFailed{Message: "the probe declares only commands"}
	}
	err := port.RPC().Stdout(ctx, []byte(fmt.Sprintf("%s: %s", call.User, strings.Join(call.Invocation.Path, " "))))
	return &plugin.ReplyExit{}, err
}

// ProbeCommand declares a single run leaf, RPC unless a native operation is given.
func ProbeCommand(name string, placement plugin.Placement, operation *commanddecl.NativeOperation) plugin.Commands {
	var kind commanddecl.LeafKind[commanddecl.NativeOperation] = &commanddecl.RPC[commanddecl.NativeOperation]{}
	summary := "A group."
	if operation != nil {
		kind = &commanddecl.Native[commanddecl.NativeOperation]{Binding: *operation}
		summary = "A native group."
	}
	return plugin.Commands{
		Placement: placement,
		Tree: plugin.Declaration{
			Node: &commanddecl.Group[commanddecl.NativeOperation]{
				Name:    name,
				Summary: summary,
				Subcommands: []commanddecl.Node[commanddecl.NativeOperation]{
					&commanddecl.Leaf[commanddecl.NativeOperation]{Name: "run", Summary: "Run.", Kind: kind},
				},
			},
		},
	}
}

// WaitFile waits for a Host's fixture file to have the observed bytes, using
// filesystem events. The watch is installed before reading to avoid missed writes.
func WaitFile(ctx context.Context, path string, ready func([]byte) bool) (err error) {
	watcher, err := fsnotify.NewWatcher()
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, watcher.Close()) }()
	if err = watcher.Add(filepath.Dir(path)); err != nil {
		return err
	}
	for {
		data, readErr := os.ReadFile(path)
		if readErr == nil && ready(data) {
			return nil
		}
		if readErr != nil && !errors.Is(readErr, os.ErrNotExist) {
			return readErr
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case err := <-watcher.Errors:
			return err
		case <-watcher.Events:
		}
	}
}

// WaitCount waits for a new manager call after a scenario's observation point.
func (m *ScriptedManager) WaitCount(ctx context.Context, call string, count int) error {
	for {
		m.mu.Lock()
		changed := m.changed
		m.mu.Unlock()
		if m.Count(call) >= count {
			return nil
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-m.done:
			return net.ErrClosed
		case <-changed:
		}
	}
}
