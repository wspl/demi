package grokbuild

import (
	"context"
	"encoding/json"
	"net/http"
	"testing"
	"time"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/types"
)

func TestRunCLIIdentity(t *testing.T) {
	v := providertest.StartVendor(t)
	v.RespondAt(chatPath, chat(`{"choices":[{"delta":{"content":"hi"}}]}`))
	p, _ := fixture(t, v, nil)
	request := providertest.InferenceRequest()
	request.ModelID = "grok-4.5"
	equal(t, []provider.Event{&provider.TextDelta{Text: "hi"}, &provider.Response{}}, run(t, p, request))
	sent := v.Requests()[0]
	equal(t, "POST", sent.Method)
	equal(t, chatPath, sent.URI)
	for name, want := range map[string]string{
		"authorization":            "Bearer session-token",
		"x-xai-token-auth":         "xai-grok-cli",
		"x-authenticateresponse":   "authenticate-response",
		"x-grok-client-identifier": "grok-shell",
		"x-grok-client-mode":       "interactive",
		"x-grok-client-version":    "1.0.5",
		"x-userid":                 "user-1",
		"x-grok-user-id":           "user-1",
		"x-email":                  "user@example.com",
		"x-grok-model-override":    "grok-4.5",
		"x-grok-session-id":        "session-1",
		"x-grok-conv-id":           "session-1",
		"x-grok-req-id":            "request-1",
		"x-grok-turn-idx":          "turn-1",
		"accept":                   "text/event-stream",
	} {
		equal(t, want, sent.Header(name))
	}
	if _, present := sent.Headers[http.CanonicalHeaderKey("x-grok-client-surface")]; present {
		t.Fatal("chat request included client surface header")
	}
}

func TestChatReplayToolsEffort(t *testing.T) {
	v := providertest.StartVendor(t)
	v.RespondAt(chatPath, chat())
	p, _ := fixture(t, v, nil)
	limit := uint32(8000)
	tier := "priority"
	r := providertest.InferenceRequest()
	r.ModelID = "grok-4.5"
	r.SystemPrompt = "system"
	r.OutputLimit = &limit
	r.ServiceTierID = &tier
	r.Thinking = &types.EffortConfig{Effort: "high"}
	r.Tools = []provider.ToolDefinition{
		{
			Name:        "read_file",
			Description: "Read a file",
			InputSchema: json.RawMessage(`{"type":"object","properties":{"path":{"type":"string"}}}`),
		},
	}
	r.Items = []provider.InferenceItem{
		&provider.UserMessage{Content: []provider.UserPart{&provider.TextPart{Text: "hello"}}},
		&provider.AssistantThinking{ModelID: r.ModelID, Text: "hidden"},
		&provider.AssistantText{ModelID: r.ModelID, Text: "Use tool"},
		&provider.ToolUse{
			ModelID:   r.ModelID,
			ToolUseID: "call-1",
			ToolName:  "read_file",
			Input:     json.RawMessage(`{"path":"a.ts"}`),
		},
		&provider.ToolResult{
			ToolUseID: "call-1",
			Output: []provider.ResultPart{
				&provider.TextPart{Text: "contents"},
				&provider.ResultImage{Bytes: provider.MediaBytes{Data: []byte("PNG"), MediaType: "image/png"}},
			},
		},
	}
	run(t, p, r)
	equal(
		t,
		jsonValue(
			t,
			`{"model":"grok-4.5","messages":[{"role":"system",`+
				`"content":"system"},{"role":"user","content":"hello"},`+
				`{"role":"assistant","content":"Use tool",`+
				`"tool_calls":[{"id":"call-1","type":"function",`+
				`"function":{"name":"read_file","arguments":"{\"path\":\"a.ts\"}"}`+
				`}]},{"role":"tool","tool_call_id":"call-1",`+
				`"content":"contents\n[image:image/png]"}],"stream":true,`+
				`"stream_options":{"include_usage":true},`+
				`"tools":[{"type":"function","function":{"name":"read_file",`+
				`"description":"Read a file","parameters":{"type":"object",`+
				`"properties":{"path":{"type":"string"}}}}}],"tool_choice":"auto",`+
				`"reasoning_effort":"high"}`,
		),
		v.Requests()[0].JSON(t),
	)
}

func TestVideoAndPDFAttachments(t *testing.T) {
	v := providertest.StartVendor(t)
	v.RespondAt(chatPath, chat())
	p, _ := fixture(t, v, nil)
	r := providertest.InferenceRequest()
	r.Items = []provider.InferenceItem{
		&provider.UserMessage{
			Content: []provider.UserPart{
				&provider.TextPart{Text: "look"},
				&provider.VideoPart{Medium: &provider.MediaBytes{Data: []byte{1, 2}, MediaType: "video/mp4"}},
				&provider.DocumentPart{
					FileName: "a.pdf",
					Bytes:    provider.MediaBytes{Data: []byte("%PDF"), MediaType: "application/pdf"},
				},
				&provider.ImagePart{Medium: &provider.MediaURL{URL: "https://example.com/shot.png"}},
			},
		},
	}
	run(t, p, r)
	body := v.Requests()[0].JSON(t).(map[string]any)
	equal(
		t,
		jsonValue(
			t,
			`[{"role":"user","content":[{"type":"text","text":"look"},`+
				`{"type":"text","text":"[video:video/mp4]"},{"type":"image_url",`+
				`"image_url":{"url":"https://example.com/shot.png","detail":"auto"}}]}]`,
		),
		body["messages"],
	)
}

func TestStreamReasoningToolsUsage(t *testing.T) {
	v := providertest.StartVendor(t)
	v.RespondAt(
		chatPath,
		chat(
			`{"choices":[{"delta":{"reasoning_content":"think"}}]}`,
			`{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1",`+
				`"function":{"name":"shell_exec","arguments":"{\"cmd\""}}]}}]}`,
			`{"choices":[{"delta":{"tool_calls":[{"index":0,`+
				`"function":{"arguments":":\"ls\"}"}}]},"finish_reason":"tool_calls"}]}`,
			`{"choices":[],"usage":{"prompt_tokens":5,"completion_tokens":2}}`,
		),
	)
	p, _ := fixture(t, v, nil)
	equal(
		t,
		[]provider.Event{
			&provider.ThinkingStart{},
			&provider.ThinkingDelta{Text: "think"},
			&provider.ToolCall{ToolUseID: "c1", ToolName: "shell_exec", Input: json.RawMessage(`{"cmd":"ls"}`)},
			&provider.Response{Usage: types.TokenUsage{InputTokens: 5, OutputTokens: 2}},
		},
		run(t, p, providertest.InferenceRequest()),
	)
}

func TestRefusedRequestRecord(t *testing.T) {
	v := providertest.StartVendor(t)
	response := answer(429, "slow down")
	response.Headers.Set("Retry-After", "7")
	v.RespondAt(chatPath, response)
	p, _ := fixture(t, v, nil)
	events := run(t, p, providertest.InferenceRequest())
	if len(events) != 1 {
		t.Fatalf("events: %v", events)
	}
	failure := events[0].(*provider.Error).Failure
	equal(t, "Grok Build API request failed with HTTP 429: slow down", failure.Message)
	equal(t, provider.RateLimit, *failure.Code)
	equal(t, 7*time.Second, *failure.RetryAfter)
	if failure.Diagnostics == nil || failure.Diagnostics.Upstream == nil {
		t.Fatal("missing vendor record")
	}
	record := jsonValue(t, *failure.Diagnostics.Upstream).(map[string]any)
	equal(t, "slow down", record["body"])
	equal(t, float64(http.StatusTooManyRequests), record["status"])
}

func TestCancelStream(t *testing.T) {
	v := providertest.StartVendor(t)
	response := providertest.EventStream("data: {\"choices\":[{\"delta\":{\"content\":\"hel\"}}]}\n\n")
	response.Ending = providertest.Open
	v.RespondAt(chatPath, response)
	p, _ := fixture(t, v, nil)
	r, err := p.Runtime(provider.RuntimeEnv{HTTP: p.http})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := r.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	reader := providertest.NewEventReader(
		ctx,
		t,
		func(ctx context.Context) provider.Run {
			return r.Run(ctx, providertest.InferenceRequest())
		},
	)
	event, ok := reader.NextEvent()
	equal(t, true, ok)
	equal(t, provider.Event(&provider.TextDelta{Text: "hel"}), event)
	cancel()
	_, ok = reader.NextEvent()
	equal(t, false, ok)
	v.Disconnected(t.Context())
}
