package backendtest_test

import (
	"bytes"
	"fmt"
	"io/fs"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/go/backendtest"
	"github.com/wspl/demi/go/backendtest/scripted"
)

// configuredNamed is a configured model of the id, with the efforts low and high
// and a fast tier.
func configuredNamed(id string) backendtest.Map {
	return backendtest.Map{
		"id": id, "displayName": id + " display", "contextWindow": 64_000, "outputLimit": 4_000,
		"thinkingEfforts": []any{"low", "high"}, "acceptedExtensions": []any{"pdf"}, "fastTier": "priority",
	}
}

func createProvider(b *backendtest.Backend, session *backendtest.Session, body backendtest.Map) map[string]any {
	return b.Post("/api/providers", session, body).Expect(http.StatusCreated).At("provider").(map[string]any)
}

func modelsOf(b *backendtest.Backend, session *backendtest.Session, query string) *backendtest.Answer {
	return b.Get("/api/models"+query, session).Expect(http.StatusOK)
}

func providerList(b *backendtest.Backend, session *backendtest.Session) []any {
	list, _ := b.Get("/api/providers", session).Expect(http.StatusOK).At("providers").([]any)
	return list
}

// dataHolds reports whether any file of the data directory holds the bytes.
func dataHolds(t *testing.T, h *backendtest.Harness, secret string) bool {
	t.Helper()
	found := false
	err := filepath.WalkDir(h.DataDir(), func(path string, entry fs.DirEntry, err error) error {
		if err != nil || !entry.Type().IsRegular() {
			return err
		}
		content, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		if bytes.Contains(content, []byte(secret)) {
			found = true
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	return found
}

// Cost: one backend, about a second.
func TestAnAPIKeyEntryIsSealedAtRestAndAnsweredWithoutItsKey(t *testing.T) {
	t.Parallel()
	h := backendtest.New(t, backendtest.WithMode("isolated"))
	b, master := h.StartSetUp()
	created := createProvider(b, master, backendtest.Map{
		"source": "custom", "providerType": "anthropic", "label": " Work ", "apiKey": "sk-ant-secret-1",
		"baseUrl": "https://proxy.example/v1", "models": []any{configuredNamed("claude-work")},
	})
	if created["kind"] != "api_key" || created["providerType"] != "anthropic" || created["label"] != "Work" ||
		created["baseUrl"] != "https://proxy.example/v1" || created["wireApi"] != nil || created["vendorId"] != nil ||
		backendtest.At(created, "models.0.id") != "claude-work" {
		t.Fatalf("the created entry is %v", created)
	}
	listed := b.Get("/api/providers", master)
	backendtest.AssertJSON(t, listed.At("providers"), []any{created})
	if strings.Contains(listed.Text(), "sk-ant-secret-1") {
		t.Fatal("the list shows the key")
	}
	// The data directory holds the configuration sealed. Every file of it is
	// read as bytes, so the check does not depend on how the backend stores it.
	if dataHolds(t, h, "sk-ant-secret-1") {
		t.Fatal("the data directory holds the key in the clear")
	}

	for _, refusal := range []struct {
		body backendtest.Map
		code string
	}{
		{backendtest.Map{"source": "custom", "providerType": "nope", "label": "x", "apiKey": "k"}, "unknown_provider_type"},
		{backendtest.Map{"source": "custom", "providerType": "anthropic", "wireApi": "responses", "label": "x", "apiKey": "k"}, "invalid_body"},
		{backendtest.Map{"source": "custom", "providerType": "anthropic", "label": "x", "apiKey": "k", "models": []any{configuredNamed("m"), configuredNamed("m")}}, "invalid_body"},
		{backendtest.Map{"source": "custom", "providerType": "anthropic", "label": "x", "apiKey": "two\nlines"}, "invalid_body"},
		{backendtest.Map{"source": "custom", "providerType": "anthropic", "label": "  ", "apiKey": "k"}, "invalid_body"},
		{backendtest.Map{"providerType": "anthropic", "label": "x", "apiKey": "k"}, "invalid_body"},
	} {
		wantRefusal(t, b.Post("/api/providers", master, refusal.body), http.StatusBadRequest, refusal.code, jsonText(refusal.body))
	}

	// An edit replaces the key and returns to the family's endpoint and the live
	// catalog; the answer never shows the key.
	path := "/api/providers/" + created["id"].(string)
	edited := b.Patch(path, master, backendtest.Map{"label": "Personal", "apiKey": "sk-ant-secret-2", "baseUrl": nil, "models": nil}).Expect(http.StatusOK)
	if strings.Contains(edited.Text(), "sk-ant-secret") {
		t.Fatal("the edit's answer shows the key")
	}
	if edited.Str("provider.label") != "Personal" || edited.At("provider.baseUrl") != nil || edited.At("provider.models") != nil {
		t.Fatalf("the edited entry is %s", edited.Body)
	}
	if got := modelsOf(b, master, "").Str("providers.0.models.0.id"); got != "claude-opus-4-8" {
		t.Fatalf("the anthropic directory starts with %q", got)
	}

	// API-key entries of one family repeat.
	repeated := backendtest.Map{"source": "custom", "providerType": "anthropic", "label": "Second", "apiKey": "k"}
	second, third := createProvider(b, master, repeated), createProvider(b, master, repeated)
	if second["id"] == third["id"] {
		t.Fatal("two entries share an id")
	}
	for _, extra := range []map[string]any{second, third} {
		b.Delete("/api/providers/"+extra["id"].(string), master)
	}

	// Another user of an isolated instance reaches none of it.
	alice := b.CreateUser(master, "alice@example.test", "alice-pass-1", "user")
	if got := providerList(b, alice); len(got) != 0 {
		t.Fatalf("alice lists %v", got)
	}
	wantRefusal(t, b.Delete(path, alice), http.StatusNotFound, "provider_not_found", "alice deleting the master's entry")
	wantRefusal(t, b.Get(path+"/status", alice), http.StatusNotFound, "provider_not_found", "alice reading the master's entry")

	b.Delete(path, master).Expect(http.StatusNoContent)
	if got := providerList(b, master); len(got) != 0 {
		t.Fatalf("the master lists %v", got)
	}
	if got, _ := modelsOf(b, master, "").At("providers").([]any); len(got) != 0 {
		t.Fatalf("the catalog lists %v", got)
	}
	wantRefusal(t, b.Patch(path, master, backendtest.Map{"label": "Gone"}), http.StatusNotFound, "provider_not_found", "editing a deleted entry")
	b.Stop()
}

// Cost: two backends' starts, one scripted vendor, about two seconds.
func TestOnASharedInstanceOnlyTheMasterConfiguresAndEveryoneInfersWithItsEntries(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	h := backendtest.New(t)
	b, master := h.StartSetUp()
	admin := b.CreateUser(master, "admin@example.test", "admin-pass-1", "admin")
	bob := b.CreateUser(master, "bob@example.test", "bob-pass-1", "user")
	body := backendtest.Map{
		"source": "custom", "providerType": "anthropic", "label": "Instance", "apiKey": "k",
		"baseUrl": vendor.URL("/v1"), "models": []any{configuredNamed("m")},
	}
	for _, user := range []*backendtest.Session{admin, bob} {
		wantRefusal(t, b.Post("/api/providers", user, body), http.StatusForbidden, "forbidden", "an entry of a user")
	}
	shared := createProvider(b, master, body)
	backendtest.AssertJSON(t, providerList(b, bob), []any{shared})
	models := modelsOf(b, bob, "")
	if models.Str("providers.0.providerId") != shared["id"] {
		t.Fatalf("bob's catalog is %s", models.Body)
	}
	backendtest.AssertJSON(t, models.At("providers.0.availability"), backendtest.Map{"type": "available"})
	path := "/api/providers/" + shared["id"].(string)
	wantRefusal(t, b.Delete(path, bob), http.StatusForbidden, "forbidden", "bob deleting the entry")
	wantRefusal(t, b.Post(path+"/test", bob, backendtest.Map{"modelId": "m"}), http.StatusForbidden, "forbidden", "bob testing the entry")
	// A user's conversation infers with the instance's entry.
	b.CreateConversation(bob, convFirst)
	b.Choose(bob, convFirst, shared["id"].(string), "m")
	socket := b.Connect(bob, convFirst)
	socket.Open()
	vendor.Respond(scripted.Answer([]string{"ok"}, 1, 1))
	turn := socket.Chat("m1", "hello")
	if !strings.Contains(jsonText(turn), `"ok"`) {
		t.Fatalf("the turn is %v", frameTypes(turn))
	}
	socket.Close()
	b.Stop()

	// The entries stay the master's own under the other mode, and nobody else's:
	// another user's conversation cannot open with them, nor select them.
	h.SetMode("isolated")
	b = h.Start()
	master = b.Login(backendtest.MasterEmail, backendtest.MasterPassword)
	bob = b.Login("bob@example.test", "bob-pass-1")
	socket = b.Connect(bob, convFirst)
	socket.Send(backendtest.Frame{"type": "open"})
	if frame := socket.Frame(); frame["type"] != "error" || frame["code"] != "provider_not_found" {
		t.Fatalf("another user's entry is not the user's to open with: %v", frame)
	}
	chosen := b.Patch("/api/conversations/"+convFirst, bob, backendtest.Map{"model": backendtest.Map{"providerId": shared["id"], "modelId": "m"}})
	wantRefusal(t, chosen, http.StatusNotFound, "provider_not_found", "selecting the master's entry")
	socket.Close()
	if got := providerList(b, master); len(got) != 1 {
		t.Fatalf("the master lists %v", got)
	}
	if got := providerList(b, bob); len(got) != 0 {
		t.Fatalf("bob lists %v", got)
	}
	b.Stop()
}

func modelsDevDocument() backendtest.Map {
	return backendtest.Map{
		"deepseek": backendtest.Map{
			"id": "deepseek", "name": "DeepSeek", "npm": "@ai-sdk/openai-compatible",
			"api": "https://api.deepseek.com", "doc": "https://api-docs.deepseek.com",
			"models": backendtest.Map{
				"deepseek-v4": backendtest.Map{
					"name": "DeepSeek V4", "reasoning": true, "tool_call": true, "attachment": false,
					"limit": backendtest.Map{"context": 128_000, "output": 32_000},
					"cost":  backendtest.Map{"input": 0.3, "output": 1.2, "cache_read": 0.03, "cache_write": 0},
				},
				"deepseek-v4-flash": backendtest.Map{"name": "DeepSeek V4 Flash", "limit": backendtest.Map{"context": 128_000}},
			},
		},
		"minimax": backendtest.Map{
			"id": "minimax", "name": "MiniMax", "npm": "@ai-sdk/anthropic", "api": "https://api.minimax.io/anthropic/v1",
			"models": backendtest.Map{"minimax-m3": backendtest.Map{"name": "MiniMax M3", "limit": backendtest.Map{"context": 200_000, "output": 64_000}}},
		},
		"zai": backendtest.Map{"id": "zai", "name": "z.ai", "npm": "@ai-sdk/google", "models": backendtest.Map{}},
		"amazon-bedrock": backendtest.Map{
			"id": "amazon-bedrock", "name": "Amazon Bedrock", "npm": "@ai-sdk/amazon-bedrock",
			"models": backendtest.Map{"anthropic.claude-sonnet": backendtest.Map{"name": "Claude Sonnet on Bedrock"}},
		},
		"github-copilot": backendtest.Map{
			"id": "github-copilot", "name": "GitHub Copilot", "npm": "@ai-sdk/openai-compatible",
			"api": "https://api.githubcopilot.com", "models": backendtest.Map{"gpt-5.5": backendtest.Map{"name": "GPT-5.5"}},
		},
	}
}

func served(document backendtest.Map) *scripted.Response {
	return scripted.Status(200).Header("etag", `"fixture"`).Chunk(backendtest.Marshal(document))
}

// Cost: one backend and a scripted models.dev, about a second.
func TestAVendorEntryTakesItsFamilyWireAndEndpointFromModelsDevAndReadsItsLiveModels(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t, backendtest.WithModelsDev(vendor.URL("/api.json"))).StartSetUp()
	vendor.Respond(served(modelsDevDocument()))
	// Only vendors a family speaks to, by name: Bedrock's package has no family,
	// and Copilot is excluded by name. A vendor that names no endpoint and has no
	// official one starts without.
	offered := b.Get("/api/providers/catalog", master).Expect(http.StatusOK)
	backendtest.AssertJSON(t, offered.At("vendors"), []any{
		backendtest.Map{
			"id": "deepseek", "name": "DeepSeek", "providerType": "openai", "wireApi": "chat-completions",
			"baseUrl": "https://api.deepseek.com", "doc": "https://api-docs.deepseek.com",
		},
		backendtest.Map{"id": "minimax", "name": "MiniMax", "providerType": "anthropic", "baseUrl": "https://api.minimax.io/anthropic/v1", "doc": nil},
		backendtest.Map{"id": "zai", "name": "z.ai", "providerType": "google", "baseUrl": nil, "doc": nil},
	})
	wantRefusal(t, b.Post("/api/providers", master, backendtest.Map{"source": "vendor", "vendorId": "amazon-bedrock", "label": "x", "apiKey": "k"}),
		http.StatusBadRequest, "unknown_vendor", "a vendor no family speaks to")

	deepseek := createProvider(b, master, backendtest.Map{"source": "vendor", "vendorId": "deepseek", "label": "DeepSeek", "apiKey": "sk-1"})
	if deepseek["providerType"] != "openai" || deepseek["wireApi"] != "chat-completions" || deepseek["vendorId"] != "deepseek" ||
		deepseek["baseUrl"] != "https://api.deepseek.com/" {
		t.Fatalf("the vendor entry is %v", deepseek)
	}
	// The entry's catalog is the vendor's live list, read again from models.dev
	// with the copy's validator.
	vendor.Respond(scripted.Status(304))
	live := modelsOf(b, master, "")
	if got := vendor.Requests()[1].Header.Get("If-None-Match"); got != `"fixture"` {
		t.Fatalf("the validator is %q", got)
	}
	models, _ := live.At("providers.0.models").([]any)
	var ids []any
	for _, model := range models {
		ids = append(ids, backendtest.At(model, "id"))
	}
	backendtest.AssertJSON(t, ids, []any{"deepseek-v4", "deepseek-v4-flash"})
	v4 := models[0]
	if backendtest.At(v4, "contextWindow") != 128_000.0 || backendtest.At(v4, "outputLimit") != 32_000.0 ||
		backendtest.At(v4, "supportsReasoning") != true {
		t.Fatalf("deepseek-v4 is %v", v4)
	}
	if backendtest.At(v4, "selection.providerId") != deepseek["id"] || backendtest.At(v4, "selection.model.contextWindow") != 128_000.0 {
		t.Fatalf("its selection is %v", backendtest.At(v4, "selection"))
	}
	if live.At("providers.0.stale") != false {
		t.Fatal("the vendor's list is stale")
	}

	// A typed list replaces the live one, and removing it returns to it.
	path := "/api/providers/" + deepseek["id"].(string)
	b.Patch(path, master, backendtest.Map{"models": []any{configuredNamed("deepseek-v4")}})
	typed := modelsOf(b, master, "?refresh=true")
	if got, _ := typed.At("providers.0.models").([]any); len(got) != 1 {
		t.Fatalf("the typed list is %v", got)
	}
	if got := len(vendor.Requests()); got != 2 {
		t.Fatalf("a configured list is fetched: %d requests", got)
	}
	b.Patch(path, master, backendtest.Map{"models": nil})
	vendor.Respond(scripted.Status(304))
	again, _ := modelsOf(b, master, "").At("providers.0.models").([]any)
	if len(again) != 2 {
		t.Fatalf("the live list is %v", again)
	}
	b.Stop()
}

// tested tests a new entry of the family, on the wire when it names one, whose
// vendor streams the text, and answers the request the vendor received.
func tested(t *testing.T, b *backendtest.Backend, master *backendtest.Session, vendor *scripted.Vendor, family, wire, stream string) scripted.Request {
	t.Helper()
	body := backendtest.Map{
		"source": "custom", "providerType": family, "label": family, "apiKey": "sk-2",
		"baseUrl": vendor.URL("/v1"), "models": []any{configuredNamed("m")},
	}
	if wire != "" {
		body["wireApi"] = wire
	}
	entry := createProvider(b, master, body)
	vendor.Respond(scripted.EventStream(stream))
	result := b.Post("/api/providers/"+entry["id"].(string)+"/test", master, backendtest.Map{"modelId": "m"}).Expect(http.StatusOK)
	backendtest.AssertJSON(t, result.Value(), backendtest.Map{"type": "passed", "model": "m display"})
	requests := vendor.Requests()
	return requests[len(requests)-1]
}

// sseBody is a server-sent events body with one data frame per payload.
func sseBody(payloads ...backendtest.Map) string {
	var text strings.Builder
	for _, payload := range payloads {
		fmt.Fprintf(&text, "data: %s\n\n", backendtest.Marshal(payload))
	}
	return text.String()
}

// Cost: one backend and a scripted vendor, about a second.
func TestTheProviderTestSendsOneRealRequestThroughTheEntrysFamily(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t).StartSetUp()
	entry := createProvider(b, master, backendtest.Map{
		"source": "custom", "providerType": "anthropic", "label": "Work", "apiKey": "sk-ant-test", "baseUrl": vendor.URL("/v1"),
	})
	path := "/api/providers/" + entry["id"].(string) + "/test"
	answered := scripted.Message([][]scripted.Frame{scripted.TextBlock(0, []string{"ok"})}, "end_turn", scripted.Frame{"input_tokens": 3, "output_tokens": 1}, 1)
	vendor.Respond(answered)
	passed := b.Post(path, master, backendtest.Map{"modelId": "claude-opus-4-8"}).Expect(http.StatusOK)
	backendtest.AssertJSON(t, passed.Value(), backendtest.Map{"type": "passed", "model": "Claude Opus 4.8"})
	sent := vendor.Requests()[0]
	if sent.Path != "/v1/messages" || sent.Header.Get("X-Api-Key") != "sk-ant-test" {
		t.Fatalf("the request is %s with key %q", sent.Path, sent.Header.Get("X-Api-Key"))
	}
	// No later request extends a connection test, so nothing in it is marked for
	// the vendor's cache.
	body := sent.JSON(t)
	backendtest.AssertJSON(t, backendtest.At(body, "system"), []any{backendtest.Map{"type": "text", "text": "Reply with the word ok."}})
	if strings.Contains(jsonText(body), "cache_control") {
		t.Fatalf("the test's request is marked for the cache: %s", jsonText(body))
	}

	vendor.Respond(scripted.Status(401).Chunk([]byte(`{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}`)))
	refused := b.Post(path, master, backendtest.Map{"modelId": "claude-opus-4-8"}).Expect(http.StatusOK)
	if refused.Str("type") != "failed" || refused.Str("model") != "Claude Opus 4.8" ||
		!strings.Contains(refused.Str("message"), "HTTP 401") || !strings.Contains(refused.Str("message"), "invalid x-api-key") {
		t.Fatalf("the vendor refused the key: %s", refused.Body)
	}

	unknown := b.Post(path, master, backendtest.Map{"modelId": "no-such-model"}).Expect(http.StatusOK)
	if unknown.Str("type") != "failed" || unknown.Str("message") != "This provider lists no model no-such-model" || unknown.At("model") != nil {
		t.Fatalf("an unknown model gives %s", unknown.Body)
	}
	account := b.Post(path, master, backendtest.Map{"modelId": "claude-opus-4-8", "credentialId": "cred-1"})
	wantRefusal(t, account, http.StatusNotFound, "account_not_found", "a test with an account of an API-key entry")
	if got := len(vendor.Requests()); got != 2 {
		t.Fatalf("the vendor got %d requests", got)
	}

	// The other API-key families build from their entries alike: an openai entry
	// on the wire it names or on Responses, and a google entry.
	compatible := sseBody(backendtest.Map{"choices": []any{backendtest.Map{"delta": backendtest.Map{"content": "ok"}}}}) + "data: [DONE]\n\n"
	completed := sseBody(backendtest.Map{"type": "response.completed", "response": backendtest.Map{"usage": backendtest.Map{"input_tokens": 1, "output_tokens": 1}}})
	chat := tested(t, b, master, vendor, "openai", "chat-completions", compatible)
	if chat.Path != "/v1/chat/completions" || chat.Header.Get("Authorization") != "Bearer sk-2" {
		t.Fatalf("the chat request is %s with %q", chat.Path, chat.Header.Get("Authorization"))
	}
	if responses := tested(t, b, master, vendor, "openai", "", completed); responses.Path != "/v1/responses" {
		t.Fatalf("the responses request is %s", responses.Path)
	}
	google := tested(t, b, master, vendor, "google", "", "")
	if google.Path != "/v1/models/m:streamGenerateContent" || google.Header.Get("X-Goog-Api-Key") != "sk-2" {
		t.Fatalf("the google request is %s with %q", google.Path, google.Header.Get("X-Goog-Api-Key"))
	}
	b.Stop()
}
