package server_test

import (
	"context"
	"fmt"
	"slices"
	"strings"
	"sync"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/agent/server"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/agent/tools"
	"github.com/wspl/demi/internal/agent/tools/toolstest"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

const cloudContext = "The conversation now runs on the Cloud."

type testContext struct {
	mu   sync.Mutex
	seen [][]string
}

func (*testContext) Name() string { return "execution" }
func (c *testContext) Context(_ context.Context, _ tools.NodeContext, _ core.TurnID, seen []string) (*string, error) {
	c.mu.Lock()
	c.seen = append(c.seen, append([]string{}, seen...))
	c.mu.Unlock()
	if slices.Contains(seen, cloudContext) {
		return nil, nil
	}
	return new(cloudContext), nil
}
func product(contextSource *testContext, profiles ...core.Profile) func(*server.Deps[*toolstest.NoHost]) {
	return func(deps *server.Deps[*toolstest.NoHost]) {
		commands := &host.CommandSet{}
		err := commands.Register(host.Group("greet", "Greets the caller.", host.Leaf(declare.Leaf[declare.NativeOperation]{Name: "hello", Summary: "Say hello.", Kind: &declare.RPC[declare.NativeOperation]{}}, host.RPCHandlerFunc(func(context.Context, host.RPCInvocation, host.RPCPort) (uint8, error) { return 0, nil }))))
		if err != nil {
			panic(err)
		}
		deps.Toolsets = tools.Toolset{Commands: commands, Profiles: profiles, Revision: "test"}
		if contextSource != nil {
			deps.Context = []tools.ContextSource{contextSource}
		}
	}
}
func TestPromptIncludesHelpAndContextIsSavedBeforeRequest(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		memory := storetest.NewMemoryTreeStore()
		script := providertest.NewScriptedRuntime(t, providertest.Respond(func(_ provider.InferenceRequest) []provider.Event {
			blocks := memory.Checkpoint(rootID()).Transcript
			equal(t, 2, len(blocks))
			if _, ok := blocks[1].(*core.ContextBlock); !ok {
				t.Error("context not saved before request")
			}
			return []provider.Event{providertest.Text("answer"), providertest.Response(1, 1)}
		}), said("again"))
		source := &testContext{}
		f := fixtureWith(t, script, memory, server.DefaultConfig(), product(source))
		c := f.opened()
		c.Send(t.Context(), send("m1", "hi"))
		untilIdle(t, c)
		r := script.Requests()[0]
		if !strings.HasPrefix(r.SystemPrompt, "system prompt\n") || !strings.Contains(r.SystemPrompt, "greet: Greets the caller.") || !strings.Contains(r.SystemPrompt, "greet hello") {
			t.Fatal(r.SystemPrompt)
		}
		equal(t, 2, len(r.Items))
		c.Send(t.Context(), send("m2", "again"))
		untilIdle(t, c)
		source.mu.Lock()
		seen := source.seen
		source.mu.Unlock()
		equal(t, [][]string{{}, {cloudContext}}, seen)
	})
}

func TestProfilesAndSpawnRestrictionShapeCommands(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		first, second := make(chan struct{}), make(chan struct{})
		explore := providertest.NewScriptedRuntime(t, said("explored"))
		restricted := providertest.NewScriptedRuntime(t, held(first, providertest.Text("restricted done"), providertest.Response(1, 1)), held(second, providertest.Text("more done"), providertest.Response(1, 1)))
		rootScript := providertest.NewScriptedRuntime(t, said("noted"), said("noted"), said("noted"))
		profile := core.Profile{Name: "explorer", Description: "Reads, never edits.", Instructions: new("explorer prompt"), CanSpawnSubagents: false}
		f := fixtureWith(t, rootScript, storetest.NewMemoryTreeStore(), server.DefaultConfig(), product(nil, profile))
		f.resolver.ProvideRuntime("stub", &nodeScripts{root: rootScript, children: []childScript{{"task explore", explore}, {"task restricted", restricted}}})
		f.opened()
		for _, name := range []string{"nope", "default"} {
			r := agent(t, f, rootID(), "spawn", fmt.Sprintf(`{"prompt":"task x","profile":%q}`, name))
			equal(t, fmt.Sprintf("demi agent spawn: unknown profile %q (available: explorer)\n", name), r.stderr)
		}
		child := spawn(t, f, rootID(), `{"prompt":"task explore","profile":"explorer"}`)
		synctest.Wait()
		equal(t, "explorer", *f.store.Record(child).Profile)
		r := explore.Requests()[0]
		if !strings.HasPrefix(r.SystemPrompt, "explorer prompt\n") || strings.Contains(r.SystemPrompt, "demi agent spawn") || !strings.Contains(r.SystemPrompt, "demi agent send") {
			t.Fatal(r.SystemPrompt)
		}
		if !strings.Contains(requestText(r), "This session may not spawn subagents.") {
			t.Fatal(requestText(r))
		}
		id := spawn(t, f, rootID(), `{"prompt":"task restricted","no-subagents":true}`)
		if _, err := agentCall(t.Context(), f, id, "spawn", `{"prompt":"task grandchild"}`, false); err == nil {
			t.Fatal("restricted child has spawn command")
		}
		equal(t, uint8(0), agent(t, f, id, "list", `{}`).code)
		close(first)
		synctest.Wait()
		number := f.store.Record(id).Number
		run := agent(t, f, rootID(), "resume", fmt.Sprintf(`{"id":%d,"message":"more"}`, number))
		equal(t, uint8(0), run.code)
		if _, err := agentCall(t.Context(), f, id, "spawn", `{"prompt":"task grandchild"}`, false); err == nil {
			t.Fatal("resume lifted restriction")
		}
		close(second)
		synctest.Wait()
		if f.store.Record(id).CanSpawnSubagents {
			t.Fatal("restriction was not stored")
		}
		reserved := fixtureWith(t, providertest.NewScriptedRuntime(t), storetest.NewMemoryTreeStore(), server.DefaultConfig(), product(nil, core.Profile{Name: "default", CanSpawnSubagents: true}))
		c := reserved.client()
		c.Send(t.Context(), &framewire.OpenFrame{})
		frames := c.Received()
		if len(frames) != 1 {
			t.Fatal(frames)
		}
		if _, ok := frames[0].(*framewire.ErrorFrame); !ok {
			t.Fatal(frames)
		}
	})
}
func TestGrandchildInheritsProfileAndReadsOwnContext(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		outerGate, innerGate := make(chan struct{}), make(chan struct{})
		outerScript := providertest.NewScriptedRuntime(t, held(outerGate, providertest.Text("delegated"), providertest.Response(1, 1)), said("outer done"))
		innerScript := providertest.NewScriptedRuntime(t, held(innerGate, providertest.Text("inner done"), providertest.Response(1, 1)))
		rootScript := providertest.NewScriptedRuntime(t, said("noted"))
		model := storetest.ModelOf("stub", "worker-model")
		profile := core.Profile{Name: "worker", Description: "Works through a task list.", Instructions: new("worker prompt"), Commands: new([][]string{}), CanSpawnSubagents: true, Model: &model}
		source := &testContext{}
		f := fixtureWith(t, rootScript, storetest.NewMemoryTreeStore(), server.DefaultConfig(), product(source, profile))
		f.resolver.ProvideRuntime("stub", &nodeScripts{root: rootScript, children: []childScript{{"task outer", outerScript}, {"task inner", innerScript}}})
		f.opened()
		outer := spawn(t, f, rootID(), `{"prompt":"task outer","profile":"worker"}`)
		done := make(chan core.NodeID, 1)
		go func() { done <- spawn(t, f, outer, `{"prompt":"task inner"}`) }()
		synctest.Wait()
		close(outerGate)
		inner := <-done
		synctest.Wait()
		equal(t, uint8(0), agent(t, f, inner, "list", `{}`).code)
		r := innerScript.Requests()[0]
		if !strings.HasPrefix(r.SystemPrompt, "worker prompt\n") || strings.Contains(r.SystemPrompt, "greet") || !strings.Contains(r.SystemPrompt, "demi agent send") {
			t.Fatal(r.SystemPrompt)
		}
		equal(t, "worker-model", r.ModelID)
		close(innerGate)
		synctest.Wait()
		for _, id := range []core.NodeID{rootID(), outer, inner} {
			count := 0
			for _, block := range f.store.Checkpoint(id).Transcript {
				if _, ok := block.(*core.ContextBlock); ok {
					count++
				}
			}
			equal(t, 1, count)
		}
	})
}
