package backend_test

import (
	"context"
	"errors"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/backend/plugins"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/host/hosttest"
	"github.com/wspl/demi/internal/plugin"
)

// registryProbe declares the registry scenario's probe plugin.
func registryProbe(id string, commands ...plugin.Commands) *backendtest.CommandProbe {
	return &backendtest.CommandProbe{
		Declaration: plugin.Manifest{ID: plugin.ID(id), Name: id, Description: "A probe.", Commands: commands},
	}
}

// TestPluginGroupsComposeAndCallsReceiveOwnPath uses a local backend and command RPCs, with no vendor or runner.
func TestPluginGroupsComposeAndCallsReceiveOwnPath(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	h.Config.Plugins = []plugin.Factory{
		registryProbe("notes", backendtest.ProbeCommand("notes", plugin.PlacementDemi, nil)),
		registryProbe("lint", backendtest.ProbeCommand("lint", plugin.PlacementRoot, nil)),
	}
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	shard, err := b.Backend.Shards().Of(ctx, s.User.ID)
	wireMust(t, err)
	product := host.Group(
		"agent",
		"A group.",
		host.Leaf(
			declare.Leaf[declare.NativeOperation]{
				Name:    "run",
				Summary: "Run.",
				Kind:    &declare.RPC[declare.NativeOperation]{},
			},
			host.RPCHandlerFunc(func(context.Context, host.RPCInvocation, host.RPCPort) (uint8, error) {
				return 0, errors.New("manifest data must not be invoked")
			}),
		),
	)
	set, err := shard.Plugins().Toolset(ctx, []host.Declared{product})
	wireMust(t, err)
	var roots []string
	for _, node := range set.Commands.Declarations() {
		roots = append(roots, declare.Name(node))
	}
	if !reflect.DeepEqual(roots, []string{"demi", "lint"}) {
		t.Fatalf("roots: %v", roots)
	}
	help := set.Commands.RenderHelp()
	for _, want := range []string{"agent: A group.", "notes: A group."} {
		if !strings.Contains(help, want) {
			t.Fatalf("missing %q: %s", want, help)
		}
	}
	for _, path := range [][]string{{"demi", "notes", "run"}, {"lint", "run"}} {
		memory := hosttest.NewMemoryPort(nil)
		_, err := set.Commands.Dispatch(
			ctx,
			host.RPCInvocation{Path: path, Args: []byte(`{}`), CWD: "/workspace", Context: hosttest.CommandContext()},
			memory.Port(),
		)
		wireMust(t, err)
		own := path
		if path[0] == "demi" {
			own = path[1:]
		}
		want := string(s.User.ID) + ": " + strings.Join(own, " ")
		if string(memory.Stdout()) != want {
			t.Fatalf("output %q, want %q", memory.Stdout(), want)
		}
	}
	wireMust(t, b.Close(ctx))
}

// TestPluginInvalidManifestStopsStartupAndNamesPlugin uses six invalid manifests to refuse assembled startup
// before serving or starting a runner.
func TestPluginInvalidManifestStopsStartupAndNamesPlugin(t *testing.T) {
	t.Parallel()
	profile := func(id, name string) *backendtest.CommandProbe {
		p := registryProbe(id)
		p.Declaration.Profiles = []core.Profile{{Name: name, Description: "A profile.", CanSpawnSubagents: true}}
		return p
	}
	cases := []struct {
		factories []plugin.Factory
		kind      plugins.RegistryErrorKind
		id        plugin.ID
		prefix    string
	}{
		{
			[]plugin.Factory{registryProbe("todo"), registryProbe("todo")},
			plugins.DuplicateID,
			"todo",
			`two plugins have the id "todo"`,
		},
		{
			[]plugin.Factory{registryProbe("todo", backendtest.ProbeCommand("agent", plugin.PlacementDemi, nil))},
			plugins.TakenCommand,
			"todo",
			`plugin "todo" declares "demi agent", which is taken`,
		},
		{
			[]plugin.Factory{
				registryProbe("one", backendtest.ProbeCommand("notes", plugin.PlacementDemi, nil)),
				registryProbe("two", backendtest.ProbeCommand("notes", plugin.PlacementDemi, nil)),
			},
			plugins.TakenCommand,
			"two",
			`plugin "two" declares "demi notes", which is taken`,
		},
		{
			[]plugin.Factory{
				registryProbe("one", backendtest.ProbeCommand("lint", plugin.PlacementRoot, nil)),
				registryProbe("two", backendtest.ProbeCommand("lint", plugin.PlacementRoot, nil)),
			},
			plugins.RefusedCommands,
			"two",
			`plugin "two"'s commands are refused`,
		},
		{
			[]plugin.Factory{profile("todo", "default")},
			plugins.InvalidProfile,
			"todo",
			`plugin "todo" declares the profile "default", which is reserved for inheriting the parent`,
		},
		{
			[]plugin.Factory{profile("one", "explorer"), profile("two", "explorer")},
			plugins.InvalidProfile,
			"two",
			`plugin "two" declares the profile "explorer", which another plugin declares`,
		},
	}
	for _, scenario := range cases {
		t.Run(scenario.prefix, func(t *testing.T) {
			t.Parallel()
			ctx, h := conversationHarness(t)
			h.Config.Plugins = scenario.factories
			b, err := h.Start(ctx, t)
			if err == nil {
				wireMust(t, b.Close(ctx))
				t.Fatal("invalid manifest started")
			}
			var refused *plugins.RegistryError
			if !errors.As(err, &refused) || refused.Kind != scenario.kind || refused.Plugin != scenario.id {
				t.Fatalf("startup: %v", err)
			}
			if !strings.HasPrefix(refused.Error(), scenario.prefix) {
				t.Fatalf("diagnostic: %v", refused)
			}
		})
	}
}

// TestPluginUnservedNativeTreeIsOmittedWhole publishes the file package, but executes no command or model.
func TestPluginUnservedNativeTreeIsOmittedWhole(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	built, err := backendtest.BuildPackage(ctx, t, "demi-file")
	wireMust(t, err)
	wireMust(t, h.UsePackage(ctx, built))
	p := registryProbe(
		"tools",
		backendtest.ProbeCommand(
			"served",
			plugin.PlacementDemi,
			&declare.NativeOperation{Package: "demi.file", Operation: "file.read"},
		),
		backendtest.ProbeCommand(
			"unserved",
			plugin.PlacementDemi,
			&declare.NativeOperation{Package: "demi.missing", Operation: "run"},
		),
	)
	p.Declaration.Profiles = []core.Profile{{Name: "explorer", Description: "A profile.", CanSpawnSubagents: true}}
	h.Config.Plugins = []plugin.Factory{p}
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	shard, err := b.Backend.Shards().Of(ctx, s.User.ID)
	wireMust(t, err)
	set, err := shard.Plugins().Toolset(ctx, nil)
	wireMust(t, err)
	if len(set.Profiles) != 1 || set.Profiles[0].Name != "explorer" {
		t.Fatalf("profiles: %+v", set.Profiles)
	}
	help := set.Commands.RenderHelp()
	if !strings.Contains(help, "served: A native group.") || strings.Contains(help, "unserved") {
		t.Fatalf("help: %s", help)
	}
	wireMust(t, b.Close(ctx))
}
