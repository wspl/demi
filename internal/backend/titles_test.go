package backend_test

import (
	"context"
	"fmt"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/providers/openaiapi"
	"github.com/wspl/demi/internal/webapi"
)

// conversationFamily makes the scripted runtime available through provider routes.
type conversationFamily struct {
	build func(providers.FamilyArgs) provider.Runtime
}

func (conversationFamily) Credential() webapi.CredentialKind { return webapi.CredentialKindAPIKey }
func (conversationFamily) Wires() []core.WireAPI             { return nil }
func (f conversationFamily) Provider(args providers.FamilyArgs) (provider.Provider, error) {
	if _, ok := args.Credential.(*providers.APIKeyArgs); !ok {
		return nil, &providers.FamilyError{Kind: providers.FamilyWrongCredential}
	}
	return &conversationProvider{Provider: openaiapi.New(openaiapi.Config{APIKey: "fixture"}, args.Clock), runtime: f.build(args)}, nil
}

type conversationProvider struct {
	*openaiapi.Provider
	runtime provider.Runtime
}

func (p *conversationProvider) Runtime(provider.RuntimeEnv) (provider.Runtime, error) {
	return p.runtime.Fresh(), nil
}

// titleRuntime separates title requests from turns, and gates only title answers.
type titleRuntime struct {
	asked   chan provider.InferenceRequest
	answers chan struct{}
	answer  string
}

func (r *titleRuntime) Run(ctx context.Context, request provider.InferenceRequest) provider.Run {
	return func(yield func(provider.Event) bool) {
		text := "Answer."
		if strings.HasPrefix(request.SystemPrompt, "You are a title generator") {
			select {
			case r.asked <- request:
			case <-ctx.Done():
				return
			}
			select {
			case <-r.answers:
			case <-ctx.Done():
				return
			}
			text = r.answer
		}
		for event := range providertest.Events(providertest.Text(text), providertest.Response(1, 1))(ctx, request) {
			if !yield(event) {
				return
			}
		}
	}
}
func (r *titleRuntime) Fresh() provider.Runtime   { return r }
func (*titleRuntime) Close(context.Context) error { return nil }
func (*titleRuntime) RequestLimits(core.Model) provider.RequestLimits {
	return provider.RequestLimits{}
}

// conversationConfigured describes the fixture's explicit model settings.
func conversationConfigured(output int) string {
	return fmt.Sprintf(`{"id":"m","displayName":"M","contextWindow":100000,"outputLimit":%d,"thinkingEfforts":[],"acceptedExtensions":null,"fastTier":null}`, output)
}

// conversationTitling starts titles enabled with a controllable scripted family.
func conversationTitling(t *testing.T, answer string) (context.Context, *backendtest.TestBackend, backendtest.Session, string, *titleRuntime) {
	t.Helper()
	ctx, h := conversationHarness(t)
	script := &titleRuntime{asked: make(chan provider.InferenceRequest, 4), answers: make(chan struct{}, 4), answer: answer}
	h.Config.Families.Register("titling", conversationFamily{build: func(providers.FamilyArgs) provider.Runtime { return script }})
	h.Config.Conversations.Titles = true
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	body := `{"source":"custom","providerType":"titling","label":"T","apiKey":"k","models":[` + conversationConfigured(8000) + `]}`
	entry := conversationDecode(t, conversationRequest(ctx, t, b, &s, "POST", "/api/providers", body, 201), webapi.DecodeProviderAnswer)
	return ctx, b, s, string(entry.Provider.ID), script
}

// titleAsked waits for the title request itself, rather than polling its count.
func titleAsked(ctx context.Context, t *testing.T, r *titleRuntime) provider.InferenceRequest {
	t.Helper()
	select {
	case request := <-r.asked:
		conversationEqual(t, len(r.asked), 0)
		return request
	case <-ctx.Done():
		t.Fatal(ctx.Err())
		return provider.InferenceRequest{}
	}
}

// titleInput observes the sole user message passed to the title generator.
func titleInput(t *testing.T, r provider.InferenceRequest) string {
	t.Helper()
	if len(r.Items) != 1 {
		t.Fatal("title input must be one message")
	}
	message, ok := r.Items[0].(*provider.UserMessage)
	if !ok || len(message.Content) != 1 {
		t.Fatal("title input must be one text")
	}
	text, ok := message.Content[0].(*provider.TextPart)
	if !ok {
		t.Fatal("title input is not text")
	}
	return text.Text
}

// titleUntil follows persisted title changes on the user's page channel.
func titleUntil(ctx context.Context, t *testing.T, page *backendtest.SyncChannel, want func(webapi.ConversationSummary) bool) webapi.ConversationSummary {
	t.Helper()
	events, err := page.Until(ctx, func(e webapi.SyncEvent) bool {
		c, ok := e.(*webapi.SyncEventConversation)
		return ok && c.Conversation.ID == conversationFirst && want(c.Conversation)
	})
	wireMust(t, err)
	return events[len(events)-1].(*webapi.SyncEventConversation).Conversation
}

func TestGeneratedTitleLosesToRenameAndArchive(t *testing.T) {
	t.Parallel()
	const generated = "TS2307 after package split"
	ctx, b, s, entry, script := conversationTitling(t, "\""+generated+"\"\n")
	conversationCreate(ctx, t, b, &s, conversationFirst)
	conversationChoose(ctx, t, b, &s, conversationFirst, entry, "m")
	path := "/api/conversations/" + conversationFirst
	conversationRequest(ctx, t, b, &s, "PATCH", path, `{"title":"New conversation"}`, 200)
	page, _ := conversationPage(ctx, t, b, &s)
	socket := conversationOpen(ctx, t, b, &s, conversationFirst)
	message := "why does pnpm build fail with TS2307 after I moved auth into its own package"
	_, err := socket.Chat(ctx, "m1", message)
	wireMust(t, err)
	asked := titleAsked(ctx, t, script)
	summary := conversationSummary(ctx, t, b, &s, conversationFirst)
	conversationEqual(t, summary.Title, message)
	conversationEqual(t, summary.TitleGenerating, true)
	script.answers <- struct{}{}
	titleUntil(ctx, t, page, func(c webapi.ConversationSummary) bool { return c.Title == generated })
	titled := conversationSummary(ctx, t, b, &s, conversationFirst)
	conversationEqual(t, titled.Title, generated)
	conversationEqual(t, titled.TitleGenerating, false)
	conversationEqual(t, titled.TitleCurrent, true)
	conversationEqual(t, titleInput(t, asked), "1. "+message)
	conversationEqual(t, *asked.MaxOutputTokens(), uint32(1024))
	if len(asked.Tools) != 0 || asked.Thinking != nil {
		t.Fatal("title request has tools or thinking")
	}
	totals := conversationDecode(t, conversationRequest(ctx, t, b, &s, "GET", "/api/usage", "", 200), webapi.DecodeUsageTotals)
	conversationEqual(t, totals.Totals[0].Requests, uint64(2))
	_, err = socket.Chat(ctx, "m2", "and the login test")
	wireMust(t, err)
	conversationEqual(t, conversationSummary(ctx, t, b, &s, conversationFirst).TitleCurrent, false)
	conversationRequest(ctx, t, b, &s, "POST", path+"/title", `{}`, 202)
	asked = titleAsked(ctx, t, script)
	conversationEqual(t, titleInput(t, asked), "1. "+message+"\n2. and the login test")
	conversationRequest(ctx, t, b, &s, "PATCH", path, `{"title":"Kept"}`, 200)
	script.answers <- struct{}{}
	titleUntil(ctx, t, page, func(c webapi.ConversationSummary) bool { return c.Title == "Kept" && !c.TitleGenerating })
	kept := conversationSummary(ctx, t, b, &s, conversationFirst)
	conversationEqual(t, kept.Title, "Kept")
	conversationEqual(t, kept.TitleGenerating, false)
	conversationEqual(t, kept.TitleCurrent, true)
	conversationRequest(ctx, t, b, &s, "POST", path+"/title", `{}`, 202)
	titleAsked(ctx, t, script)
	conversationRequest(ctx, t, b, &s, "PATCH", path, `{"archived":true}`, 200)
	titleUntil(ctx, t, page, func(c webapi.ConversationSummary) bool { return c.Archived && !c.TitleGenerating })
	archived := conversationDecode(t, conversationRequest(ctx, t, b, &s, "GET", "/api/conversations?archived=true", "", 200), webapi.DecodeConversations)
	if len(archived.Conversations) == 0 || archived.Conversations[0].TitleGenerating {
		t.Fatal("archived title request still generating")
	}
	conversationEqual(t, len(script.answers), 0)
	conversationRefusal(t, conversationRequest(ctx, t, b, &s, "POST", path+"/title", `{}`, 409), webapi.ErrorCodeConversationArchived)
	conversationCreate(ctx, t, b, &s, conversationSecond)
	second := "/api/conversations/" + conversationSecond + "/title"
	conversationRefusal(t, conversationRequest(ctx, t, b, &s, "POST", second, `{}`, 409), webapi.ErrorCodeModelNotSelected)
	conversationChoose(ctx, t, b, &s, conversationSecond, entry, "m")
	conversationRefusal(t, conversationRequest(ctx, t, b, &s, "POST", second, `{}`, 409), webapi.ErrorCodeNoMessages)
	conversationRequest(ctx, t, b, &s, "DELETE", "/api/providers/"+entry, "", 204)
	conversationRefusal(t, conversationRequest(ctx, t, b, &s, "POST", second, `{}`, 404), webapi.ErrorCodeProviderNotFound)
}

func TestEmptyGeneratedTitleKeepsMessageTitleAndAllowsRetry(t *testing.T) {
	t.Parallel()
	ctx, b, s, entry, script := conversationTitling(t, " \n\n")
	conversationCreate(ctx, t, b, &s, conversationFirst)
	conversationChoose(ctx, t, b, &s, conversationFirst, entry, "m")
	page, _ := conversationPage(ctx, t, b, &s)
	socket := conversationOpen(ctx, t, b, &s, conversationFirst)
	_, err := socket.Chat(ctx, "m1", "hello   there")
	wireMust(t, err)
	titleAsked(ctx, t, script)
	titleUntil(ctx, t, page, func(c webapi.ConversationSummary) bool { return c.TitleGenerating })
	script.answers <- struct{}{}
	titleUntil(ctx, t, page, func(c webapi.ConversationSummary) bool { return !c.TitleGenerating })
	left := conversationSummary(ctx, t, b, &s, conversationFirst)
	conversationEqual(t, left.TitleGenerating, false)
	conversationEqual(t, left.Title, "hello there")
	conversationEqual(t, left.TitleCurrent, false)
}
