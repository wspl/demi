package pluginhost_test

import (
	"bytes"
	"fmt"
	"log/slog"
	"reflect"
	"strconv"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/pluginhost"
	"github.com/wspl/demi/internal/cmddecl"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/types"
)

// These in-process boundary scenarios take milliseconds and use no external resources.
func TestRegistryRefusesConflicts(t *testing.T) {
	cases := []struct {
		name   string
		change func(*plugin.Manifest, *plugin.Manifest)
		want   string
	}{
		{"duplicate id", func(a, b *plugin.Manifest) {
			b.ID = a.ID
		}, `two plugins have the id "`},
		{
			"inherited profile",
			func(_, b *plugin.Manifest) {
				b.Profiles = []types.Profile{{Name: types.ProfileInherit}}
			},
			`" declares the profile "`,
		},
		{
			"duplicate profile",
			func(a, b *plugin.Manifest) {
				a.Profiles = []types.Profile{{Name: "worker"}}
				b.Profiles = a.Profiles
			},
			`" declares the profile "`,
		},
		{"duplicate stream even when unserved", func(a, b *plugin.Manifest) {
			a.Streams = []plugin.Stream{
				{
					Name:      "live",
					Operation: cmddecl.NativeOperation{Package: "missing", Operation: "live"},
					Sends:     a.Page.User.Schema,
					Receives:  a.Page.User.Schema,
				},
			}
			b.Streams = a.Streams
		}, `" declares the user stream "`},
		{
			"page package",
			func(a, b *plugin.Manifest) {
				b.Page.Package = a.Page.Package
			},
			`'s page package "`,
		},
		{
			"user topic",
			func(_, b *plugin.Manifest) {
				b.Page.User.Topics = []plugin.Topic{plugin.TopicJobs}
			},
			`, a topic of another scope`,
		},
		{
			"conversation topic",
			func(_, b *plugin.Manifest) {
				b.Page.Conversation.Topics = []plugin.Topic{plugin.TopicExposes}
			},
			`, a topic of another scope`,
		},
		{"duplicate group", func(a, b *plugin.Manifest) {
			a.Commands = []plugin.Commands{command("notes", plugin.PlacementDemi, nil)}
			b.Commands = a.Commands
		}, `" declares "demi`},
		{"duplicate root", func(a, b *plugin.Manifest) {
			a.Commands = []plugin.Commands{command("notes", plugin.PlacementRoot, nil)}
			b.Commands = a.Commands
		}, `'s commands are refused: `},
		{"demi root reserved even before groups", func(a, _ *plugin.Manifest) {
			a.Commands = []plugin.Commands{command("demi", plugin.PlacementRoot, nil)}
		}, `" declares "demi`},
		{
			"execution id",
			func(_, b *plugin.Manifest) {
				b.ID = "execution"
			},
			`'s commands are refused: `,
		},
		{"malformed command", func(_, b *plugin.Manifest) {
			b.Commands = []plugin.Commands{command("bad name", plugin.PlacementDemi, nil)}
		}, `'s commands are refused: `},
		{"conflict hidden by missing catalog", func(a, b *plugin.Manifest) {
			operation := cmddecl.NativeOperation{Package: "missing", Operation: "run"}
			a.Commands = []plugin.Commands{command("notes", plugin.PlacementDemi, &operation)}
			b.Commands = a.Commands
		}, `" declares "demi`},
	}
	for _, group := range []string{"agent", "shell", "host"} {
		cases = append(cases, struct {
			name   string
			change func(*plugin.Manifest, *plugin.Manifest)
			want   string
		}{
			"reserved " + group,
			func(_, b *plugin.Manifest) {
				b.Commands = []plugin.Commands{command(group, plugin.PlacementDemi, nil)}
			},
			`" declares "demi`,
		})
	}
	for _, scenario := range cases {
		t.Run(scenario.name, func(t *testing.T) {
			a, b := manifest(t, "a"), manifest(t, "b")
			scenario.change(&a, &b)
			_, err := pluginhost.NewRegistry(
				[]plugin.Factory{&fakeFactory{manifest: a}, &fakeFactory{manifest: b}},
				func(cmddecl.NativeOperation) bool {
					return false
				},
			)
			if err == nil || !strings.Contains(err.Error(), scenario.want) {
				t.Fatalf("got %v, want %q", err, scenario.want)
			}
			prefixes := map[string]string{
				"duplicate id":    `two plugins have the id "a"`,
				"reserved agent":  `plugin "b" declares "demi agent", which is taken`,
				"duplicate group": `plugin "b" declares "demi notes", which is taken`,
				"duplicate root":  `plugin "b"'s commands are refused`,
				"inherited profile": `plugin "b" declares the profile "default", which is reserved for ` +
					`inheriting the parent`,
				"duplicate profile": `plugin "b" declares the profile "worker", which another plugin declares`,
			}
			if prefix, ok := prefixes[scenario.name]; ok && !strings.HasPrefix(err.Error(), prefix) {
				t.Fatalf("diagnostic %q lacks %q", err, prefix)
			}
			if !strings.Contains(err.Error(), strconv.Quote(string(a.ID))) &&
				!strings.Contains(err.Error(), strconv.Quote(string(b.ID))) {
				t.Fatalf("diagnostic does not name plugin: %v", err)
			}
		})
	}
}

func TestRegistryCatalogAndOrder(t *testing.T) {
	var logs bytes.Buffer
	previous := slog.Default()
	slog.SetDefault(slog.New(slog.NewJSONHandler(&logs, nil)))
	t.Cleanup(func() {
		slog.SetDefault(previous)
	})
	served := cmddecl.NativeOperation{Package: "available", Operation: "run"}
	missing := cmddecl.NativeOperation{Package: "available", Operation: "missing"}
	a, b := manifest(t, "z-first"), manifest(t, "a-second")
	a.Profiles = []types.Profile{
		{
			Name:     "z-profile",
			Commands: new([][]string{{"demi", "notes"}}),
		},
	}
	b.Profiles = []types.Profile{{Name: "a-profile"}}
	mixed := command("mixed", plugin.PlacementDemi, &served)
	mixed.Tree.Node.(*cmddecl.Group[cmddecl.NativeOperation]).Subcommands = append(
		mixed.Tree.Node.(*cmddecl.Group[cmddecl.NativeOperation]).Subcommands,
		&cmddecl.Leaf[cmddecl.NativeOperation]{
			Name:    "missing",
			Summary: "Missing.",
			Kind:    &cmddecl.Native[cmddecl.NativeOperation]{Binding: missing},
		},
	)
	a.Commands = []plugin.Commands{mixed, command("kept", plugin.PlacementDemi, &served)}
	a.Streams = []plugin.Stream{
		{
			Name:      "gone",
			Operation: missing,
			Sends:     a.Page.User.Schema,
			Receives:  a.Page.User.Schema,
		},
		{
			Name:      "live",
			Operation: served,
			Sends:     a.Page.User.Schema,
			Receives:  a.Page.User.Schema,
		},
	}
	a.Page.User.Operations = []cmddecl.NativeOperation{missing}
	a.Page.Conversation.Operations = []cmddecl.NativeOperation{served}
	a.Page.Methods[0].Operations = []cmddecl.NativeOperation{missing}
	b.Page.Conversation.Operations = []cmddecl.NativeOperation{missing}
	a.Page.Methods[1].Operations = []cmddecl.NativeOperation{{Package: "method-only", Operation: "run"}}
	r, err := pluginhost.NewRegistry(
		[]plugin.Factory{&fakeFactory{manifest: a}, &fakeFactory{manifest: b}},
		func(operation cmddecl.NativeOperation) bool {
			return operation.Operation != "missing"
		},
	)
	if err != nil {
		t.Fatal(err)
	}
	for _, text := range []string{
		`"plugin":"z-first"`,
		`"what":"command","name":"mixed"`,
		`"what":"user stream","name":"gone"`,
		`"what":"page state","name":"user"`,
		`"what":"page state","name":"conversation"`,
		`"what":"page method","name":"user"`,
	} {
		if !strings.Contains(logs.String(), text) {
			t.Fatalf("missing catalog log %s in %s", text, logs.String())
		}
	}
	if got := r.ContextSources(); !reflect.DeepEqual(got, []plugin.ID{"z-first", "a-second"}) {
		t.Fatal(got)
	}
	if got := r.Profiles(); len(got) != 2 || got[0].Name != "z-profile" || got[1].Name != "a-profile" {
		t.Fatal(got)
	}
	// A caller and the factory cannot mutate the registry's accepted snapshot.
	(*a.Profiles[0].Commands)[0][1] = "mutated"
	exposed := r.Profiles()
	(*exposed[0].Commands)[0][1] = "also-mutated"
	if got := (*r.Profiles()[0].Commands)[0][1]; got != "notes" {
		t.Fatal(got)
	}
	if got := r.Streams(); len(got) != 1 || got[0].Name != "live" {
		t.Fatal(got)
	}
	if got := r.Followers(plugin.TopicExposes); !reflect.DeepEqual(got, []plugin.ID{"a-second"}) {
		t.Fatal(got)
	}
	if got := r.Followers(plugin.TopicJobs); !reflect.DeepEqual(got, []plugin.ID{"z-first"}) {
		t.Fatal(got)
	}
	u, _ := userFixture(t, r)
	set, err := u.Toolset(t.Context(), nil)
	if err != nil {
		t.Fatal(err)
	}
	help := set.Commands.RenderHelp()
	if strings.Contains(help, "mixed") || !strings.Contains(help, "kept: A group.") {
		t.Fatal(help)
	}
	entries, err := u.Entries(t.Context())
	if err != nil || !reflect.DeepEqual(entries[0].Packages, []string{"available", "method-only"}) {
		t.Fatalf("%+v %v", entries, err)
	}
	if state, err := u.PageState(t.Context(), "z-first"); err != nil || state != nil {
		t.Fatalf("%s %v", state, err)
	}
	if _, err := u.PageCall(
		t.Context(),
		pluginhost.PageCall{Plugin: "z-first", Method: "user", Params: []byte(`{}`)},
	); err == nil {
		t.Fatal("unserved page method reached instance")
	}
}

func TestProfileSnapshotsCannotChangeAnotherTree(t *testing.T) {
	configs := []types.ThinkingConfig{
		&types.AdaptiveConfig{Effort: "high"},
		&types.BudgetConfig{BudgetTokens: 32},
		&types.EffortConfig{Effort: "high", Summary: new(types.ThinkingSummaryDetailed)},
		&types.DisabledConfig{},
	}
	for _, config := range configs {
		t.Run(fmt.Sprintf("%T", config), func(t *testing.T) {
			m := manifest(t, "profiles")
			m.Profiles = []types.Profile{
				{
					Name:         "profile",
					Instructions: new("Instructions."),
					Commands:     new([][]string{{"demi", "notes"}}),
					Model: &types.ModelSelection{
						ProviderID:    "provider",
						ServiceTierID: new("fast"),
						Thinking:      config,
						Model: types.Model{
							ID:                 "model",
							Name:               "Model",
							InputLimit:         new(uint32(1024)),
							OutputLimit:        new(uint32(100)),
							AcceptedExtensions: new([]types.FileExtension{types.FileExtensionPNG}),
							Thinking: []types.ThinkingCapability{
								&types.AdaptiveCapability{Efforts: []string{"high"}, DefaultEffort: new("high")},
								&types.BudgetCapability{
									MinBudgetTokens:     new(uint32(1)),
									MaxBudgetTokens:     new(uint32(100)),
									DefaultBudgetTokens: new(uint32(32)),
								},
								&types.EffortCapability{
									Efforts:        []string{"high"},
									DefaultEffort:  new("high"),
									Summaries:      []types.ThinkingSummary{types.ThinkingSummaryDetailed},
									DefaultSummary: new(types.ThinkingSummaryDetailed),
								},
								&types.DisabledCapability{},
							},
						},
					},
				},
			}
			r := registry(t, &fakeFactory{manifest: m})
			u, _ := userFixture(t, r)
			set, err := u.Toolset(t.Context(), nil)
			if err != nil {
				t.Fatal(err)
			}
			profile := &set.Profiles[0]
			*profile.Instructions = "changed"
			(*profile.Commands)[0][1] = "changed"
			model := profile.Model
			*model.ServiceTierID = "changed"
			*model.Model.InputLimit = 1
			*model.Model.OutputLimit = 1
			(*model.Model.AcceptedExtensions)[0] = types.FileExtensionPDF
			model.Model.Thinking[0].(*types.AdaptiveCapability).Efforts[0] = "changed"
			*model.Model.Thinking[0].(*types.AdaptiveCapability).DefaultEffort = "changed"
			*model.Model.Thinking[1].(*types.BudgetCapability).MinBudgetTokens = 2
			*model.Model.Thinking[1].(*types.BudgetCapability).MaxBudgetTokens = 2
			*model.Model.Thinking[1].(*types.BudgetCapability).DefaultBudgetTokens = 2
			effort := model.Model.Thinking[2].(*types.EffortCapability)
			effort.Efforts[0] = "changed"
			*effort.DefaultEffort = "changed"
			effort.Summaries[0] = types.ThinkingSummaryOff
			*effort.DefaultSummary = types.ThinkingSummaryOff
			switch c := model.Thinking.(type) {
			case *types.AdaptiveConfig:
				c.Effort = "changed"
			case *types.BudgetConfig:
				c.BudgetTokens = 1
			case *types.EffortConfig:
				c.Effort = "changed"
				*c.Summary = types.ThinkingSummaryOff
			case *types.DisabledConfig:
			}
			if got := r.Profiles(); !reflect.DeepEqual(got, m.Profiles) {
				t.Fatalf("tree mutated the shared registry: %+v", got)
			}
		})
	}
}
