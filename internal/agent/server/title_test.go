package server_test

import (
	"fmt"
	"strings"
	"testing"
	"unicode/utf8"

	"github.com/wspl/demi/internal/agent/server"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/types"
)

func TestTitleFromFirstMessage(t *testing.T) {
	equal(t, "hello world", server.TitleFromMessage("  hello\n\tworld  "))
	equal(t, "", server.TitleFromMessage(" \n\t"))
	equal(t, strings.Repeat("界", 80), server.TitleFromMessage(strings.Repeat("界", 100)))
}

func TestTitleInputKeepsFirstAndRecentMessages(t *testing.T) {
	equal(t, "", server.TitleInput(nil))
	equal(t, "1. first line\n2. second", server.TitleInput([]string{" \n", " first\nline ", "second"}))
	equal(t, "1. "+strings.Repeat("界", 400), server.TitleInput([]string{strings.Repeat("界", 500)}))
	messages := []string{"opening goal"}
	for range 19 {
		messages = append(messages, strings.Repeat("x", 400))
	}
	messages = append(messages, "latest topic")
	input := server.TitleInput(messages)
	if !strings.HasPrefix(input, "1. opening goal\n…\n") || !strings.HasSuffix(input, "21. latest topic") ||
		utf8.RuneCountInString(input) > 4002 {
		t.Fatal(input)
	}
	lines := strings.Split(input, "\n")
	kept := len(lines) - 2
	if !strings.HasPrefix(lines[2], fmt.Sprintf("%d. ", 22-kept)) {
		t.Fatal(input)
	}
}

func TestTitleUsesFirstNonblankAnswerLine(t *testing.T) {
	for _, answer := range []string{
		"\n  \"A title\"\nignored",
		"‘A title’",
		"\n  “A title”  \nignored",
		"「A title」",
		"『A title』",
	} {
		got := server.TitleFromResponse(answer)
		equal(t, "A title", got)
	}
	for _, answer := range []string{" \n", "\"\"", "「 」"} {
		if got := server.TitleFromResponse(answer); got != "" {
			t.Fatal(got)
		}
	}
	got := server.TitleFromResponse(strings.Repeat("界", 100))
	equal(t, strings.Repeat("界", 80), got)
}

func TestTitleRequestUsesLowestThinkingAndNoTools(t *testing.T) {
	model := storetest.TestModel()
	model.ServiceTierID = new("priority")
	model.Model.OutputLimit = new(uint32(32000))
	model.Model.Thinking = []types.ThinkingCapability{
		&types.BudgetCapability{},
		&types.EffortCapability{Efforts: []string{"high", "minimal", "medium"}},
	}
	script := providertest.NewScriptedRuntime(
		t,
		providertest.Events(
			providertest.Thinking("ignored"),
			providertest.Text("\"A "),
			providertest.Text("title\""),
			providertest.Response(1, 1),
		),
		providertest.Events(providertest.Error("quota", nil)),
	)
	title, err := server.Title(t.Context(), script, "conversation", "r1", model, []string{"first", "second"})
	if err != nil {
		t.Fatal(err)
	}
	equal(t, "A title", title)
	requests := script.Requests()
	equal(t, 1, len(requests))
	request := requests[0]
	equal(t, "title:r1", request.TurnID)
	equal(t, server.TitleInstruction, request.SystemPrompt)
	equal(t, uint32(1024), *request.OutputCap)
	equal(t, uint32(1024), *request.MaxOutputTokens())
	equal(t, (*string)(nil), request.ServiceTierID)
	equal(t, provider.PromptCache{}, request.PromptCache)
	equal(
		t,
		[]provider.InferenceItem{
			&provider.UserMessage{Content: []provider.UserPart{&provider.TextPart{Text: "1. first\n2. second"}}},
		},
		request.Items,
	)
	equal(t, 0, len(request.Tools))
	effort, ok := request.Thinking.(*types.EffortConfig)
	if !ok {
		t.Fatal(request.Thinking)
	}
	equal(t, &types.EffortConfig{Effort: "minimal"}, effort)
	model.Model.OutputLimit = new(uint32(256))
	_, err = server.Title(t.Context(), script, "conversation", "r2", model, []string{"first"})
	if err == nil || err.Error() != "quota" {
		t.Fatal(err)
	}
	equal(t, uint32(256), *script.Requests()[1].MaxOutputTokens())
	title, err = server.Title(t.Context(), script, "conversation", "r3", model, []string{" \n"})
	if err != nil || title != "" {
		t.Fatalf("%v %v", title, err)
	}
	equal(t, 2, len(script.Requests()))
	var _ provider.Runtime = script
}
