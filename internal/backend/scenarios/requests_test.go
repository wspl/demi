package scenarios_test

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/url"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/session/sessiontest"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/agent/transcript/transcripttest"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/anthropicapi"
	"github.com/wspl/demi/internal/provider/codex"
	"github.com/wspl/demi/internal/provider/codex/codextest"
	"github.com/wspl/demi/internal/provider/google"
	"github.com/wspl/demi/internal/provider/grokbuild"
	"github.com/wspl/demi/internal/provider/openaiapi"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/types"
)

// wireTee sends each real session request to the vendor before playing its answer.
type wireTee struct{ script, real provider.Runtime }

// Run sends the vendor request before yielding the scripted answer.
func (wt *wireTee) Run(ctx context.Context, request provider.InferenceRequest) provider.Run {
	return func(yield func(provider.Event) bool) {
		for range wt.real.Run(ctx, request) {
		}
		for event := range wt.script.Run(ctx, request) {
			if !yield(event) {
				return
			}
		}
	}
}

// Fresh starts fresh runtimes for both sides of the request.
func (wt *wireTee) Fresh() provider.Runtime {
	return &wireTee{script: wt.script.Fresh(), real: wt.real.Fresh()}
}

// Close closes both runtimes and joins their errors.
func (wt *wireTee) Close(ctx context.Context) error {
	return errors.Join(wt.script.Close(ctx), wt.real.Close(ctx))
}

// RequestLimits uses the real vendor runtime’s request limits.
func (wt *wireTee) RequestLimits(model types.Model) provider.RequestLimits {
	return wt.real.RequestLimits(model)
}

type wireFamily string

const wireNow types.Timestamp = "2026-09-18T14:00:00.000Z"

func (wf wireFamily) signature(name string) string {
	switch wf {
	case "anthropic":
		return "anthropic:" + name
	case "google":
		return "google:" + name
	case "grok":
		return "grok:" + name
	default:
		prefix := "openai:"
		if wf == "codex" || wf == "codex-websocket" {
			prefix = "codex:"
		}
		return prefix + `{"type":"reasoning","id":"rs_` + name + `","summary":[],"encrypted_content":"enc-` + name + `"}`
	}
}

func (wf wireFamily) sequence() string {
	switch wf {
	case "google":
		return "contents"
	case "responses", "codex", "codex-websocket":
		return "input"
	default:
		return "messages"
	}
}

func (wf wireFamily) thinkingFields() []string {
	switch wf {
	case "anthropic":
		return []string{"thinking", "output_config"}
	case "responses", "codex", "codex-websocket":
		return []string{"reasoning"}
	case "google":
		return []string{"generationConfig"}
	default:
		return []string{"reasoning_effort"}
	}
}

// wireRuntime configures a provider at the local vendor using real credential readers.
func wireRuntime(
	ctx context.Context,
	t *testing.T,
	family wireFamily,
	vendor *providertest.MockVendor,
	socket *codextest.FakeWebSocket,
) provider.Runtime {
	t.Helper()
	clock := providertest.FixedClock(wireNow)
	env := provider.RuntimeEnv{HTTP: vendor.Client()}
	secret, err := provider.NewSecret("sk-test")
	wireMust(t, err)
	base, err := url.Parse(vendor.URL("/v1"))
	wireMust(t, err)
	var inferenceProvider provider.Provider
	switch family {
	case "anthropic":
		inferenceProvider = anthropicapi.New(
			anthropicapi.Config{APIKey: secret, BaseURL: base, Policy: provider.VendorPolicy{}},
			clock,
		)
	case "responses", "chat":
		wire := types.WireAPIResponses
		if family == "chat" {
			wire = types.WireAPIChatCompletions
		}
		inferenceProvider = openaiapi.New(
			openaiapi.Config{APIKey: secret, BaseURL: base, Wire: wire, Policy: provider.VendorPolicy{}},
			clock,
		)
	case "google":
		base, err = url.Parse(vendor.URL("/v1beta"))
		wireMust(t, err)
		inferenceProvider = google.New(google.Config{APIKey: secret, BaseURL: base}, clock)
	case "codex", "codex-websocket":
		token := providertest.JWT(
			t,
			json.RawMessage(`{"exp":1789743600,"https://api.openai.com/auth":{"chatgpt_account_id":"acct-1"}}`),
		)
		tokenJSON, err := contract.EncodeJSON(token)
		wireMust(t, err)
		document := `{"accessToken":` + string(
			tokenJSON,
		) + `,"refreshToken":"refresh-1","idToken":` + string(
			tokenJSON,
		) + `,"accountId":"acct-1","lastRefresh":"` + string(
			wireNow,
		) + `"}`
		id := "cred-c"
		pool := wirePool(ctx, t, id, document)
		config := codex.Config{Account: &id}
		config.AuthURL = vendor.URL("")
		config.BackendURL = vendor.URL("/backend-api")
		config.Transport = codex.SSE
		if family == "codex-websocket" {
			config.BackendURL = socket.BackendURL()
			config.Transport = codex.WebSocket
			env.HTTP = socket.Client()
		}
		inferenceProvider, err = codex.New(config, pool, &provider.MemorySnapshots{}, env.HTTP, clock)
		wireMust(t, err)
	case "grok":
		issuer, err := contract.EncodeJSON(vendor.URL(""))
		wireMust(t, err)
		document := `{"accessToken":"session-token","refreshToken":"refresh-1",` +
			`"expiresAt":"2030-01-01T00:00:00.000Z","issuer":` + string(
			issuer,
		) + `,"clientId":"client-1","userId":"user-1","email":"user@example.com"}`
		id := "cred-g"
		config := grokbuild.Config{Account: &id}
		config.ProxyURL = base
		config.IssuerURL, err = url.Parse(vendor.URL(""))
		wireMust(t, err)
		inferenceProvider = grokbuild.New(
			config,
			wirePool(ctx, t, id, document),
			&provider.MemorySnapshots{},
			env.HTTP,
			clock,
		)
	default:
		t.Fatalf("unknown family %s", family)
	}
	runtime, err := inferenceProvider.Runtime(env)
	wireMust(t, err)
	return runtime
}

func wirePool(ctx context.Context, t *testing.T, id, document string) *provider.MemoryCredentialPool {
	t.Helper()
	pool := provider.NewMemoryCredentialPool()
	identity := "acct-1"
	wireMust(
		t,
		pool.Write(
			ctx,
			provider.AccountMeta{
				ID:          id,
				Label:       "user@example.com",
				UpdatedAt:   wireNow,
				Source:      "test",
				IdentityKey: &identity,
			},
			document,
		),
	)
	wireMust(t, pool.SetActive(ctx, id))
	return pool
}

// wireConversation drives all six vendor requests with real sessions, tools,
// media, steering, agent messages, compaction and model switching.
func wireConversation(t *testing.T, family wireFamily) []string {
	t.Helper()
	ctx := t.Context()
	vendor := providertest.StartVendor(t)
	scripts := make([]codextest.Script, 8)
	for i := range scripts {
		vendor.Respond(providertest.EventStream(""))
		scripts[i] = codextest.Script{
			Handshake: codextest.Accept,
			Steps: []codextest.Step{
				{
					Kind: codextest.Send,
					Text: `{"type":"response.completed","response":{"usage":{"input_tokens":1,"output_tokens":1}}}`,
				},
			},
		}
	}
	socket := codextest.Start(t, scripts, "")
	answer := func(text string) providertest.Turn {
		return providertest.Events(providertest.Text(text), providertest.Response(1, 1))
	}
	script := providertest.NewScriptedRuntime(
		t,
		providertest.Events(
			&provider.ThinkingStart{},
			providertest.Thinking("thinking first"),
			&provider.ThinkingSignature{Signature: family.signature("first")},
			providertest.Text("Looking."),
			providertest.ToolCall("call-look", "look", json.RawMessage(`{}`)),
			providertest.ToolCall("call-note", "note", json.RawMessage(`{"text":"seen"}`)),
			providertest.Response(1, 1),
		),
		answer("Both seen."),
		providertest.Events(
			&provider.ThinkingStart{},
			providertest.Thinking("thinking second"),
			&provider.ThinkingSignature{Signature: family.signature("second")},
			providertest.ToolCall("call-yield", "yield", json.RawMessage(`{"durationMs":600000}`)),
			providertest.Response(1, 1),
		),
		answer(
			"The user showed a screenshot and asked twice.",
		),
		answer("After the summary."),
		answer("Thought harder."),
	)
	entered, release := make(chan struct{}), make(chan struct{})
	runtime := &sessiontest.Runtime{
		Prompt: "system prompt",
		Definitions: []provider.ToolDefinition{
			{Name: "look", InputSchema: json.RawMessage(`{"type":"object"}`)},
			{Name: "note", InputSchema: json.RawMessage(`{"type":"object"}`)},
			{Name: "yield", InputSchema: json.RawMessage(`{"type":"object"}`)},
		},
	}
	runtime.Invoke = func(ctx context.Context, call session.ToolInvocation) (session.ToolOutcome, error) {
		switch call.ToolName {
		case "look":
			close(entered)
			select {
			case <-release:
				return session.ToolOutcome{
					Output: []provider.ResultPart{
						&provider.ResultImage{
							Bytes: provider.MediaBytes{Data: storetest.PNG(3, 2, 2), MediaType: "image/png"},
						},
					},
				}, nil
			case <-ctx.Done():
				return session.ToolOutcome{}, ctx.Err()
			}
		case "yield":
			return session.ToolOutcome{Effect: &session.ScheduleYield{DurationMS: 600000}}, nil
		default:
			return session.ToolOutcome{Output: []provider.ResultPart{&provider.TextPart{Text: "noted"}}}, nil
		}
	}
	tree := storetest.NewMemoryTreeStore()
	model := storetest.ModelReading("stub", "model-a", []types.FileExtension{types.FileExtensionPNG})
	conversation := session.New(
		session.Init{
			ID:      "root",
			CWD:     "/workspace",
			Model:   model,
			Runtime: &wireTee{script: script, real: wireRuntime(ctx, t, family, vendor, socket)},
		},
		session.Deps{
			Runtime: runtime,
			Store:   tree.Session("root"),
			IDs:     transcripttest.NewSequentialIDs("id"),
			Clock:   providertest.FixedClock(types.UnixEpoch),
			Config:  session.DefaultConfig(),
		},
	)
	defer func() {
		wireMust(t, conversation.Dispose(context.Background()))
	}()
	wireMust(t, tree.CreateNode(ctx, store.RootRecord("root", types.UnixEpoch), conversation.FirstCheckpoint()))
	photo := storetest.PNG(4, 3, 1)
	held := store.HeldMedia{}
	held.Hold(types.BlobRefOf(photo), photo)
	conversation.HoldMedia(&held)
	first, err := conversation.Send(
		[]types.UserContentBlock{
			&types.UserText{Text: "What is on the screen?"},
			&types.UserImage{Source: &types.MediaSourceRef{Ref: types.BlobRefOf(photo), MediaType: "image/png"}},
		},
		"t1",
	)
	wireMust(t, err)
	select {
	case <-entered:
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	wireMust(t, conversation.Steer(storetest.Text("mind the tests"), "s1"))
	wireMust(
		t,
		conversation.AcceptAgentMessage(
			ctx,
			types.AgentMessage{
				ID:          "m1",
				Sender:      types.Sender{ID: "child", Number: 1, Description: "worker", Round: 1},
				RecipientID: "root",
				Timestamp:   types.UnixEpoch,
				Content:     "m1",
				Event:       &types.MessageEvent{},
			},
		),
	)
	close(release)
	wait := func(a *session.ActionAnswer, err error) {
		t.Helper()
		wireMust(t, err)
		_, err = a.Wait(ctx)
		wireMust(t, err)
	}
	wait(first, nil)
	wait(conversation.Send(storetest.Text("And the rest?"), "t2"))
	wait(conversation.Compact())
	wait(conversation.Send(storetest.Text("Go on."), "t3"))
	model.Thinking = &types.AdaptiveConfig{Effort: "high"}
	wireMust(t, conversation.UpdateModel(session.ModelSwitch{Model: model}))
	wait(conversation.Send(storetest.Text("Think harder."), "t4"))
	if script.Remaining() != 0 {
		t.Fatalf("unplayed turns: %d", script.Remaining())
	}
	var bodies []string
	if family == "codex-websocket" {
		for _, connection := range socket.Connections() {
			if len(connection.Received) != 1 {
				t.Fatalf("received: %#v", connection.Received)
			}
			bodies = append(bodies, connection.Received[0])
		}
	} else {
		for _, request := range vendor.Requests() {
			bodies = append(bodies, string(request.Body))
		}
	}
	if len(bodies) != 6 {
		t.Fatalf("got %d wire requests, want 6: %v", len(bodies), bodies)
	}
	return bodies
}

// wireCached compares the JSON values vendors cache, excluding Anthropic cache marks.
func wireCached(t *testing.T, family wireFamily, body string, omitThinking bool) (map[string]any, []any) {
	t.Helper()
	fields, err := contract.Object([]byte(body))
	wireMust(t, err)
	fixed := make(map[string]any)
	var parts []any
	for name, raw := range fields {
		value, err := contract.Decode[any](raw)
		wireMust(t, err)
		value = wireUnmarked(value)
		if name != family.sequence() {
			omit := false
			if omitThinking {
				for _, thinking := range family.thinkingFields() {
					omit = omit || name == thinking
				}
			}
			if !omit {
				fixed[name] = value
			}
			continue
		}
		messages, ok := value.([]any)
		if !ok {
			t.Fatalf("%s must be an array", name)
		}
		for _, message := range messages {
			if family != "anthropic" && family != "google" {
				parts = append(parts, message)
				continue
			}
			object, ok := message.(map[string]any)
			if !ok {
				t.Fatal("message must be an object")
			}
			name := "content"
			if family == "google" {
				name = "parts"
			}
			blocks, ok := object[name].([]any)
			if !ok {
				t.Fatalf("%s must be an array", name)
			}
			for _, block := range blocks {
				parts = append(parts, []any{object["role"], block})
			}
		}
	}
	return fixed, parts
}

func wireUnmarked(value any) any {
	switch value := value.(type) {
	case map[string]any:
		delete(value, "cache_control")
		for key, child := range value {
			value[key] = wireUnmarked(child)
		}
	case []any:
		for i, child := range value {
			value[i] = wireUnmarked(child)
		}
	}
	return value
}

func wireExtends(t *testing.T, family wireFamily, earlier, later string, omitThinking bool) {
	t.Helper()
	earlierFixed, earlierParts := wireCached(t, family, earlier, omitThinking)
	laterFixed, laterParts := wireCached(t, family, later, omitThinking)
	if !reflect.DeepEqual(earlierFixed, laterFixed) {
		t.Fatalf("fixed request fields changed:\nearlier %#v\nlater %#v", earlierFixed, laterFixed)
	}
	if len(laterParts) <= len(earlierParts) || !reflect.DeepEqual(laterParts[:len(earlierParts)], earlierParts) {
		t.Fatalf("request did not extend cached prefix:\nearlier %#v\nlater %#v", earlierParts, laterParts)
	}
}

func wireMust(t *testing.T, err error) {
	t.Helper()
	if err != nil {
		t.Fatal(err)
	}
}

// TestProviderRequestsPreservePrefixesAcrossSummaryAndThinking
// checks cached vendor prefixes across session changes.
// Seven conversations hit local scripted vendors, cost no model usage and take
// about one second together; all ordering uses events.
func TestProviderRequestsPreservePrefixesAcrossSummaryAndThinking(t *testing.T) {
	t.Parallel()
	// The Codex pair stays ordered: WebSocket compares SSE's captured requests.
	var overEvents []string
	for _, family := range []wireFamily{"anthropic", "responses", "chat", "google", "codex", "codex-websocket", "grok"} {
		t.Run(string(family), func(t *testing.T) {
			if family != "codex" && family != "codex-websocket" {
				t.Parallel()
			}
			bodies := wireConversation(t, family)
			if family == "codex" {
				overEvents = bodies
			}
			if family == "codex-websocket" {
				for i, body := range bodies {
					want := `{"type":"response.create",` + overEvents[i][1:]
					if body != want {
						t.Fatalf("WebSocket body %d differs from SSE:\n%s\n%s", i, body, want)
					}
				}
			}
			for i := 1; i <= 3; i++ {
				wireExtends(t, family, bodies[i-1], bodies[i], false)
			}
			thirdFixed, thirdParts := wireCached(t, family, bodies[2], false)
			_, summaryParts := wireCached(t, family, bodies[3], false)
			if len(summaryParts) != len(thirdParts)+1 ||
				!strings.Contains(
					fmt.Sprint(summaryParts[len(summaryParts)-1]),
					strings.Split(sessiontest.CompactionSummaryInstruction, ".")[0],
				) {
				t.Fatal("summary must append only its instruction")
			}
			afterFixed, afterParts := wireCached(t, family, bodies[4], false)
			if !reflect.DeepEqual(afterFixed, thirdFixed) {
				t.Fatal("compaction changed fixed fields")
			}
			system := 0
			if family == "chat" || family == "grok" {
				system = 1
			}
			if !reflect.DeepEqual(afterParts[:system], thirdParts[:system]) ||
				!strings.Contains(fmt.Sprint(afterParts[system]), "Previous conversation summary:") {
				t.Fatal("compaction did not preserve system and restart at summary")
			}
			switch family {
			case "anthropic":
				if strings.Contains(bodies[4], `"thinking"`) {
					t.Fatal("compaction retained Anthropic reasoning")
				}
			case "responses", "codex", "codex-websocket":
				if !strings.Contains(bodies[4], "rs_second") {
					t.Fatal("compaction lost reasoning")
				}
			case "google":
				if !strings.Contains(bodies[4], `"thoughtSignature":"second"`) {
					t.Fatal("compaction lost thought signature")
				}
			}
			deeperFixed, _ := wireCached(t, family, bodies[5], false)
			if reflect.DeepEqual(afterFixed, deeperFixed) {
				t.Fatal("thinking setting did not change wire fields")
			}
			wireExtends(t, family, bodies[4], bodies[5], true)
		})
	}
}
