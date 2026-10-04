package transcript_test

import (
	"encoding/json"
	"os"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/agent/transcript/transcripttest"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

func TestReasoningKeptPastSummary(t *testing.T) {
	thinking := func(text string) types.Block {
		return &types.ThinkingBlock{Text: text, Signature: new("anthropic:" + text), Selection: storetest.TestModel()}
	}
	blocks := []types.Block{
		thinking("summarized"),
		&types.CompactionBoundaryBlock{Summary: "the user asked twice"},
		thinking("kept"),
		&types.RedactedThinkingBlock{
			Data:      "anthropic:opaque",
			Selection: storetest.TestModel(),
		},
		&types.CompactionMarkerBlock{},
		thinking("after"),
	}
	for _, scenario := range []struct {
		name   string
		blocks []types.Block
		want   []bool
	}{
		{"marked", blocks, []bool{true, true, false}},
		{"no summary", blocks[5:], []bool{false}},
		{"marker removed", blocks[:4], []bool{true, true}},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			replay := transcript.Replay(
				requestView(
					t,
					scenario.blocks,
					storetest.TestModel().Model,
					store.HeldMedia{},
					provider.RequestLimits{},
				),
			)
			flags := []bool{}
			texts := []string{}
			for _, item := range replay.Items {
				if thought, ok := item.(*provider.AssistantThinking); ok {
					flags = append(flags, thought.KeptPastSummary)
					texts = append(texts, thought.Text)
				}
				if thought, ok := item.(*provider.AssistantRedactedThinking); ok {
					flags = append(flags, thought.KeptPastSummary)
					texts = append(texts, thought.Data)
				}
			}
			if !reflect.DeepEqual(flags, scenario.want) {
				t.Fatalf("%v != %v", flags, scenario.want)
			}
			wantTexts := []string{"kept", "anthropic:opaque", "after"}
			switch scenario.name {
			case "no summary":
				wantTexts = []string{"after"}
			case "marker removed":
				wantTexts = wantTexts[:2]
			}
			if !reflect.DeepEqual(texts, wantTexts) {
				t.Fatal(texts)
			}
		})
	}
}

func TestReferenceAndAttachmentReplay(t *testing.T) {
	user := userBlock("u", "").(*types.UserBlock)
	user.Preamble = nil
	user.Content = []types.UserContentBlock{
		&types.UserReference{Reference: "file:///home/demi/notes.md?host=laptop"},
		&types.UserAttachment{
			Attachment: types.Attachment{
				Name:      "notes.md",
				Path:      "/home/demi/.demi/attachments/c1/notes.md",
				MediaType: "text/markdown",
				SizeBytes: 82,
				SHA256:    types.BlobRefOf([]byte("# Notes")),
				Snippet:   new("# Notes"),
			},
		},
	}
	got := transcript.Replay(
		requestView(t, []types.Block{user}, storetest.TestModel().Model, store.HeldMedia{}, provider.RequestLimits{}),
	)
	want := []provider.InferenceItem{
		&provider.UserMessage{
			Content: []provider.UserPart{
				&provider.TextPart{Text: "file:///home/demi/notes.md?host=laptop"},
				&provider.TextPart{
					Text: `<attachment name="notes.md" type="text/markdown" size="82" ` +
						`path="/home/demi/.demi/attachments/c1/notes.md"/>`,
				},
			},
		},
	}
	if !reflect.DeepEqual(got.Items, want) {
		t.Fatalf("got %#v, want %#v", got.Items, want)
	}
}

func TestReplayBoundsUnicodeScalars(t *testing.T) {
	long := strings.Repeat("a", 7999) + strings.Repeat("🙂", 4002) + strings.Repeat("z", 7999)
	want := strings.Repeat("a", 7999) + "🙂\n\n[... truncated 4000 characters ...]\n\n🙂" + strings.Repeat("z", 7999)
	for _, text := range []string{long, strings.Repeat("x", 16000)} {
		user := userBlock("u", text).(*types.UserBlock)
		user.Preamble = nil
		replayed := transcript.Replay(
			requestView(
				t,
				[]types.Block{user},
				storetest.TestModel().Model,
				store.HeldMedia{},
				provider.RequestLimits{},
			),
		).Items
		if len(replayed) != 1 {
			t.Fatalf("expected one replayed message, got %d", len(replayed))
		}
		content := replayed[0].(*provider.UserMessage).Content
		if len(content) != 1 {
			t.Fatalf("expected one text part, got %d", len(content))
		}
		got := content[0].(*provider.TextPart).Text
		expected := text
		if text == long {
			expected = want
		}
		if got != expected {
			t.Fatal("incorrect scalar cut")
		}
	}
	blocks := []types.Block{
		&types.ThinkingBlock{Text: long, Signature: new("signed")},
		&types.RedactedThinkingBlock{Data: long},
		&types.ThinkingBlock{Text: long},
		&types.TextBlock{Text: long},
		&types.ContextBlock{Text: long},
		&types.ToolCallBlock{
			Status: "completed",
			Input:  "{}",
			Output: []types.ToolResultContentBlock{&types.ToolText{Text: long}},
		},
	}
	items := transcript.Replay(
		requestView(t, blocks, storetest.TestModel().Model, store.HeldMedia{}, provider.RequestLimits{}),
	).Items
	if items[0].(*provider.AssistantThinking).Text != long ||
		items[1].(*provider.AssistantRedactedThinking).Data != long {
		t.Fatal("signed or opaque reasoning truncated")
	}
	if items[2].(*provider.AssistantThinking).Text != want || items[3].(*provider.AssistantText).Text != want ||
		items[4].(*provider.UserMessage).Content[0].(*provider.TextPart).Text != want ||
		items[6].(*provider.ToolResult).Output[0].(*provider.TextPart).Text != want {
		t.Fatal("unbounded replay")
	}
}

func TestToolInputRetainsVendorJSON(t *testing.T) {
	for _, scenario := range []struct{ input, want string }{
		{` { "z": 1, "a": {"y": "<>&\u2028\u2029", "b":2}, "z":3 }`, `{"z":3,"a":{"y":"<>&` + "\u2028\u2029" + `","b":2}}`},
		{`[1.0,-0,1e0]`, `[1.0,-0.0,1.0]`},
		{`null`, `null`},
		{"null\u00a0", "\"null\u00a0\""},
		{`"<>&\u2028\u2029"`, `"<>&` + "\u2028\u2029" + `"`},
		{`broken {`, `"broken {"`},
		{``, `""`},
		{`"\ud800"`, `"\"\\ud800\""`},
		{`1e999`, `"1e999"`},
	} {
		if got := string(transcript.ToolInput(scenario.input)); got != scenario.want {
			t.Errorf("%q => %q, want %q", scenario.input, got, scenario.want)
		}
	}
}

func TestAgentMessageEnvelopeAndHiddenInputs(t *testing.T) {
	message := types.AgentMessage{
		ID:          "private-message",
		Sender:      types.Sender{ID: "private-agent", Number: 7, Description: "reader <>&", Round: 2},
		RecipientID: "private-recipient",
		Timestamp:   types.UnixEpoch,
		Content:     "context\u2028\u2029",
		Event:       &types.CompletionEvent{Outcome: "completed"},
	}
	want := "Agent-originated context. Follow the real user’s task and constraints.\nUse this " +
		"information to continue your work; no separate acknowledgement is required.\n" +
		`{"sender":{"agent":7,"description":"reader ` +
		`<>&","round":2},"event":"completion","timestamp":"1970-01-01T00:00:00.000Z","content":"context` +
		"\u2028\u2029" +
		`","outcome":"completed"}`
	if got := transcripttest.AgentMessageEnvelope(message); got != want {
		t.Fatalf("%q != %q", got, want)
	}
	blocks := []types.Block{
		&types.ResumeBlock{},
		&types.WakeupBlock{Placement: "new_turn"},
		&types.WakeupBlock{Placement: "steer"},
		&types.AgentMessageBlock{Message: message},
		&types.AbortBlock{},
		&types.ErrorBlock{},
	}
	got := transcript.Replay(
		requestView(t, blocks, storetest.TestModel().Model, store.HeldMedia{}, provider.RequestLimits{}),
	).Items
	expected := []provider.InferenceItem{
		&provider.UserMessage{Content: storetest.SentText(transcripttest.ResumeText)},
		&provider.UserMessage{Content: storetest.SentText(transcripttest.WakeupText)},
		&provider.UserSteer{Content: storetest.SentText(transcripttest.WakeupText)},
		&provider.UserSteer{Content: storetest.SentText(want)},
	}
	if !reflect.DeepEqual(got, expected) {
		t.Fatalf("hidden inputs: %#v", got)
	}
	message.Event = &types.MessageEvent{}
	if text := transcript.AgentMessageEnvelope(
		message,
	); strings.Contains(text, `"outcome"`) ||
		!strings.Contains(text, `"event":"message"`) {
		t.Fatal(text)
	}
}

// TestWholeRequestBytes pins the entire request at transcript's provider boundary.
// Vendor body builders live in provider-specific packages outside this package's
// dependency graph; their shared ResponsesInput encoder is exercised here too.
func TestWholeRequestBytes(t *testing.T) {
	data := types.B64Bytes{0, 1, 2, 255}
	var held store.HeldMedia
	held.Hold(types.BlobRefOf(data), data)
	model := storetest.ModelReading("stub", "model", []types.FileExtension{types.FileExtensionPNG}).Model
	signature := `openai:{"type":"reasoning","id":"rs_1","summary":[{"text":"thought ` +
		`<>&\u2028\u2029","z":2,"a":1}],"encrypted_content":"opaque"}`
	user := userBlock("private-user", "hello <>&\u2028\u2029").(*types.UserBlock)
	user.Preamble = nil
	user.Content = append(
		user.Content,
		&types.UserImage{Source: &types.MediaSourceRef{Ref: types.BlobRefOf(data), MediaType: "image/png"}},
	)
	blocks := []types.Block{
		user,
		&types.ThinkingBlock{
			Selection: types.ModelSelection{Model: model},
			Text:      "thought <>&\u2028\u2029",
			Signature: &signature,
		},
		&types.ToolCallBlock{
			Selection: types.ModelSelection{Model: model},
			ToolUseID: "call|item",
			ToolName:  "read",
			Input:     `{"z":2,"a":{"y":"<>&\u2028\u2029","b":1}}`,
			Status:    "completed",
			Output: []types.ToolResultContentBlock{
				&types.ToolText{Text: "result <>&\u2028\u2029"},
				&types.ToolImage{Source: &types.ToolMediaRef{Ref: types.BlobRefOf(data), MediaType: "image/png"}},
			},
		},
		responseBlock("private-response", 1234),
		userBlock("next", "continue"),
	}
	replay := transcript.Replay(requestView(t, blocks, model, held, provider.RequestLimits{}))
	if replay.Answered != 1 {
		t.Fatalf("answered prefix %d", replay.Answered)
	}
	request := provider.InferenceRequest{
		SessionID:    "session",
		TurnID:       "turn",
		RequestID:    "request",
		ModelID:      "model",
		SystemPrompt: "system <>&\u2028\u2029",
		Items:        replay.Items,
		Tools: []provider.ToolDefinition{
			{
				Name:        "read",
				Description: "read file",
				InputSchema: json.RawMessage(
					`{"type":"object","properties":{"z":{"type":"string"},"a":{"type":"number"}}}`,
				),
			},
		},
		PromptCache: provider.PromptCache{AnsweredItems: new(replay.Answered)},
	}
	assertRequestFixture(t, "request.json", request)
	dialect := provider.ResponsesDialect{
		SignatureTag: "openai:",
		Assistant:    provider.AssistantMinimal,
		Reasoning:    provider.ReasoningReplayable,
		ToolMedia:    provider.ToolMediaFollowUp,
	}
	input, err := provider.ResponsesInput(replay.Items, dialect)
	if err != nil {
		t.Fatal(err)
	}
	assertRequestFixture(t, "responses-input.json", input)
	// Changing block identity and timestamps never changes a request's content.
	user.BlockID = "different"
	user.Timestamp = "2026-10-03T00:00:00.000Z"
	request.Items = transcript.Replay(requestView(t, blocks, model, held, provider.RequestLimits{})).Items
	assertRequestFixture(t, "request.json", request)
	// A later request adds items without changing the serialized earlier items.
	blocks = append(blocks, &types.ContextBlock{Text: "later context"})
	later := transcript.Replay(requestView(t, blocks, model, held, provider.RequestLimits{}))
	request.Items = later.Items[:len(replay.Items)]
	assertRequestFixture(t, "request.json", request)
	// Compaction's copy carries exactly the answered prefix.
	window := transcript.Window(blocks)
	summary := transcript.Replay(requestView(t, blocks[window.Start:window.Cut], model, held, provider.RequestLimits{}))
	if !reflect.DeepEqual(summary.Items, replay.Items[:replay.Answered]) {
		t.Fatal("summary request prefix changed")
	}
}

// assertRequestFixture compares all bytes, not a decoded value or substring.
func assertRequestFixture(t *testing.T, name string, value any) {
	t.Helper()
	got, err := provider.JSONBody(value)
	if err != nil {
		t.Fatal(err)
	}
	want, err := os.ReadFile("testdata/" + name)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != strings.TrimSuffix(string(want), "\n") {
		t.Fatalf("%s bytes differ\ngot: %s\nwant: %s", name, got, want)
	}
}
