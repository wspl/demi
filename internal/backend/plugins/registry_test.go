package plugins_test

import (
	"bytes"
	"errors"
	"fmt"
	"log/slog"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/plugins"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/plugin"
)

// These in-process boundary scenarios take milliseconds and use no external resources.
func TestRegistryRefusesConflicts(t *testing.T) {
	cases := []struct {
		name   string
		change func(*plugin.Manifest, *plugin.Manifest)
		kind   plugins.RegistryErrorKind
	}{
		{"duplicate id", func(a, b *plugin.Manifest) { b.ID = a.ID }, plugins.DuplicateID},
		{"inherited profile", func(_, b *plugin.Manifest) { b.Profiles = []core.Profile{{Name: core.ProfileInherit}} }, plugins.InvalidProfile},
		{"duplicate profile", func(a, b *plugin.Manifest) { a.Profiles = []core.Profile{{Name: "worker"}}; b.Profiles = a.Profiles }, plugins.InvalidProfile},
		{"duplicate stream even when unserved", func(a, b *plugin.Manifest) {
			a.Streams = []plugin.Stream{{Name: "live", Operation: declare.NativeOperation{Package: "missing", Operation: "live"}, Sends: a.Page.User.Schema, Receives: a.Page.User.Schema}}
			b.Streams = a.Streams
		}, plugins.TakenStream},
		{"page package", func(a, b *plugin.Manifest) { b.Page.Package = a.Page.Package }, plugins.TakenPagePackage},
		{"user topic", func(_, b *plugin.Manifest) { b.Page.User.Topics = []plugin.Topic{plugin.TopicJobs} }, plugins.ForeignTopic},
		{"conversation topic", func(_, b *plugin.Manifest) { b.Page.Conversation.Topics = []plugin.Topic{plugin.TopicExposes} }, plugins.ForeignTopic},
		{"duplicate group", func(a, b *plugin.Manifest) {
			a.Commands = []plugin.Commands{command("notes", plugin.PlacementDemi, nil)}
			b.Commands = a.Commands
		}, plugins.TakenCommand},
		{"duplicate root", func(a, b *plugin.Manifest) {
			a.Commands = []plugin.Commands{command("notes", plugin.PlacementRoot, nil)}
			b.Commands = a.Commands
		}, plugins.RefusedCommands},
		{"demi root reserved even before groups", func(a, _ *plugin.Manifest) {
			a.Commands = []plugin.Commands{command("demi", plugin.PlacementRoot, nil)}
		}, plugins.TakenCommand},
		{"execution id", func(_, b *plugin.Manifest) { b.ID = "execution" }, plugins.RefusedCommands},
		{"malformed command", func(_, b *plugin.Manifest) {
			b.Commands = []plugin.Commands{command("bad name", plugin.PlacementDemi, nil)}
		}, plugins.RefusedCommands},
		{"conflict hidden by missing catalog", func(a, b *plugin.Manifest) {
			op := declare.NativeOperation{Package: "missing", Operation: "run"}
			a.Commands = []plugin.Commands{command("notes", plugin.PlacementDemi, &op)}
			b.Commands = a.Commands
		}, plugins.TakenCommand},
	}
	for _, group := range []string{"agent", "shell", "host"} {
		cases = append(cases, struct {
			name   string
			change func(*plugin.Manifest, *plugin.Manifest)
			kind   plugins.RegistryErrorKind
		}{"reserved " + group, func(_, b *plugin.Manifest) { b.Commands = []plugin.Commands{command(group, plugin.PlacementDemi, nil)} }, plugins.TakenCommand})
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			a, b := manifest(t, "a"), manifest(t, "b")
			tc.change(&a, &b)
			_, err := plugins.NewRegistry([]plugin.Factory{&fakeFactory{manifest: a}, &fakeFactory{manifest: b}}, func(declare.NativeOperation) bool { return false })
			var refused *plugins.RegistryError
			if !errors.As(err, &refused) || refused.Kind != tc.kind {
				t.Fatalf("got %v, want kind %v", err, tc.kind)
			}
			prefixes := map[string]string{
				"duplicate id":      `two plugins have the id "a"`,
				"reserved agent":    `plugin "b" declares "demi agent", which is taken`,
				"duplicate group":   `plugin "b" declares "demi notes", which is taken`,
				"duplicate root":    `plugin "b"'s commands are refused`,
				"inherited profile": `plugin "b" declares the profile "default", which is reserved for inheriting the parent`,
				"duplicate profile": `plugin "b" declares the profile "worker", which another plugin declares`,
			}
			if prefix, ok := prefixes[tc.name]; ok && !strings.HasPrefix(err.Error(), prefix) {
				t.Fatalf("diagnostic %q lacks %q", err, prefix)
			}
			if !strings.Contains(err.Error(), string(refused.Plugin)) {
				t.Fatalf("diagnostic does not name plugin: %v", err)
			}
		})
	}
}

func TestRegistryCatalogAndOrder(t *testing.T) {
	var logs bytes.Buffer
	previous := slog.Default()
	slog.SetDefault(slog.New(slog.NewJSONHandler(&logs, nil)))
	t.Cleanup(func() { slog.SetDefault(previous) })
	served := declare.NativeOperation{Package: "available", Operation: "run"}
	missing := declare.NativeOperation{Package: "available", Operation: "missing"}
	a, b := manifest(t, "z-first"), manifest(t, "a-second")
	a.Profiles = []core.Profile{{Name: "z-profile", Commands: new([][]string{{"demi", "notes"}})}}
	b.Profiles = []core.Profile{{Name: "a-profile"}}
	mixed := command("mixed", plugin.PlacementDemi, &served)
	mixed.Tree.Node.(*declare.Group[declare.NativeOperation]).Subcommands = append(mixed.Tree.Node.(*declare.Group[declare.NativeOperation]).Subcommands, &declare.Leaf[declare.NativeOperation]{Name: "missing", Summary: "Missing.", Kind: &declare.Native[declare.NativeOperation]{Binding: missing}})
	a.Commands = []plugin.Commands{mixed, command("kept", plugin.PlacementDemi, &served)}
	a.Streams = []plugin.Stream{{Name: "gone", Operation: missing, Sends: a.Page.User.Schema, Receives: a.Page.User.Schema}, {Name: "live", Operation: served, Sends: a.Page.User.Schema, Receives: a.Page.User.Schema}}
	a.Page.User.Operations = []declare.NativeOperation{missing}
	a.Page.Conversation.Operations = []declare.NativeOperation{served}
	a.Page.Methods[0].Operations = []declare.NativeOperation{missing}
	b.Page.Conversation.Operations = []declare.NativeOperation{missing}
	a.Page.Methods[1].Operations = []declare.NativeOperation{{Package: "method-only", Operation: "run"}}
	r, err := plugins.NewRegistry([]plugin.Factory{&fakeFactory{manifest: a}, &fakeFactory{manifest: b}}, func(op declare.NativeOperation) bool { return op.Operation != "missing" })
	if err != nil {
		t.Fatal(err)
	}
	for _, text := range []string{`"plugin":"z-first"`, `"what":"command","name":"mixed"`, `"what":"user stream","name":"gone"`, `"what":"page state","name":"user"`, `"what":"page state","name":"conversation"`, `"what":"page method","name":"user"`} {
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
	if _, err := u.PageCall(t.Context(), plugins.PageCall{Plugin: "z-first", Method: "user", Params: []byte(`{}`)}); err == nil {
		t.Fatal("unserved page method reached instance")
	}
}

func TestProfileSnapshotsCannotChangeAnotherTree(t *testing.T) {
	configs := []core.ThinkingConfig{
		&core.AdaptiveConfig{Effort: "high"},
		&core.BudgetConfig{BudgetTokens: 32},
		&core.EffortConfig{Effort: "high", Summary: new(core.ThinkingSummaryDetailed)},
		&core.DisabledConfig{},
	}
	for _, config := range configs {
		t.Run(fmt.Sprintf("%T", config), func(t *testing.T) {
			m := manifest(t, "profiles")
			m.Profiles = []core.Profile{{Name: "profile", Instructions: new("Instructions."), Commands: new([][]string{{"demi", "notes"}}), Model: &core.ModelSelection{
				ProviderID: "provider", ServiceTierID: new("fast"), Thinking: config,
				Model: core.Model{ID: "model", Name: "Model", InputLimit: new(uint32(1024)), OutputLimit: new(uint32(100)), AcceptedExtensions: new([]core.FileExtension{core.FileExtensionPNG}),
					Thinking: []core.ThinkingCapability{
						&core.AdaptiveCapability{Efforts: []string{"high"}, DefaultEffort: new("high")},
						&core.BudgetCapability{MinBudgetTokens: new(uint32(1)), MaxBudgetTokens: new(uint32(100)), DefaultBudgetTokens: new(uint32(32))},
						&core.EffortCapability{Efforts: []string{"high"}, DefaultEffort: new("high"), Summaries: []core.ThinkingSummary{core.ThinkingSummaryDetailed}, DefaultSummary: new(core.ThinkingSummaryDetailed)},
						&core.DisabledCapability{},
					},
				},
			}}}
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
			(*model.Model.AcceptedExtensions)[0] = core.FileExtensionPDF
			model.Model.Thinking[0].(*core.AdaptiveCapability).Efforts[0] = "changed"
			*model.Model.Thinking[0].(*core.AdaptiveCapability).DefaultEffort = "changed"
			*model.Model.Thinking[1].(*core.BudgetCapability).MinBudgetTokens = 2
			*model.Model.Thinking[1].(*core.BudgetCapability).MaxBudgetTokens = 2
			*model.Model.Thinking[1].(*core.BudgetCapability).DefaultBudgetTokens = 2
			effort := model.Model.Thinking[2].(*core.EffortCapability)
			effort.Efforts[0] = "changed"
			*effort.DefaultEffort = "changed"
			effort.Summaries[0] = core.ThinkingSummaryOff
			*effort.DefaultSummary = core.ThinkingSummaryOff
			switch c := model.Thinking.(type) {
			case *core.AdaptiveConfig:
				c.Effort = "changed"
			case *core.BudgetConfig:
				c.BudgetTokens = 1
			case *core.EffortConfig:
				c.Effort = "changed"
				*c.Summary = core.ThinkingSummaryOff
			case *core.DisabledConfig:
			}
			if got := r.Profiles(); !reflect.DeepEqual(got, m.Profiles) {
				t.Fatalf("tree mutated the shared registry: %+v", got)
			}
		})
	}
}
