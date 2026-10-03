package backend_test

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"net/http"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/webapi"
)

// configuredAccountModel encodes the typed model supplied through provider settings.
func configuredAccountModel(t *testing.T, id string) string {
	return accountJSON(t, contract.Field{Name: "id", Value: id}, contract.Field{Name: "displayName", Value: id + " display"}, contract.Field{Name: "contextWindow", Value: 64000}, contract.Field{Name: "outputLimit", Value: 4000}, contract.Field{Name: "thinkingEfforts", Value: []string{"low", "high"}}, contract.Field{Name: "acceptedExtensions", Value: []string{"pdf"}}, contract.Field{Name: "fastTier", Value: "priority"})
}

// accountProviderEntry creates a provider through its HTTP boundary.
func accountProviderEntry(ctx context.Context, t *testing.T, b *backendtest.TestBackend, s *backendtest.Session, body string) webapi.ProviderDTO {
	t.Helper()
	a := accountRequest(ctx, t, b, "POST", "/api/providers", s, body)
	accountEqual(t, a.Status, 201)
	return accountDecode(t, a, webapi.DecodeProviderAnswer).Provider
}

// accountModels reads the catalog through the same route as a model picker.
func accountModels(ctx context.Context, t *testing.T, b *backendtest.TestBackend, s *backendtest.Session, query string) webapi.ModelCatalog {
	t.Helper()
	a := accountRequest(ctx, t, b, "GET", "/api/models"+query, s, "")
	accountEqual(t, a.Status, 200)
	return accountDecode(t, a, webapi.DecodeModelCatalog)
}

func TestAnAPIKeyEntryIsSealedAtRestAndAnsweredWithoutItsKey(t *testing.T) {
	ctx := t.Context()
	h := accountHarness(ctx, t)
	h.Config.Mode = webapi.InstanceModeIsolated
	h.Config.Families, _ = backendtest.AccountFamilies(t, nil)
	h.Config.Families.Register("scripted", &backendtest.AccountFamily{T: t, Directory: &backendtest.AccountDirectory{}})
	b, master := accountStart(ctx, t, h)
	created := accountProviderEntry(ctx, t, b, &master, `{"source":"custom","providerType":"anthropic","label":" Work ","apiKey":"sk-ant-secret-1","baseUrl":"https://proxy.example/v1","models":[`+configuredAccountModel(t, "claude-work")+`]}`)
	accountEqual(t, created.Kind, webapi.CredentialKindAPIKey)
	accountEqual(t, created.ProviderType, "anthropic")
	accountEqual(t, created.Label, "Work")
	accountEqual(t, string(*created.BaseURL), "https://proxy.example/v1")
	if created.WireAPI != nil || created.VendorID != nil {
		t.Fatal(created)
	}
	accountEqual(t, (*created.Models)[0].ID, "claude-work")
	listed := accountRequest(ctx, t, b, "GET", "/api/providers", &master, "")
	accountEqual(t, accountDecode(t, listed, webapi.DecodeProviders).Providers, []webapi.ProviderDTO{created})
	if bytes.Contains(listed.Body, []byte("sk-ant-secret-1")) {
		t.Fatal("key disclosed")
	}
	db, err := h.ControlDatabase(ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	var sealed []byte
	if err := db.QueryRowContext(ctx, "SELECT config FROM providers WHERE id = ?", created.ID).Scan(&sealed); err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(sealed, []byte("sk-ant-secret-1")) {
		t.Fatal("key not sealed")
	}
	for _, row := range []struct {
		body string
		code webapi.ErrorCode
	}{{`{"source":"custom","providerType":"nope","label":"x","apiKey":"k"}`, webapi.ErrorCodeUnknownProviderType}, {`{"source":"custom","providerType":"anthropic","wireApi":"responses","label":"x","apiKey":"k"}`, webapi.ErrorCodeInvalidBody}, {`{"source":"custom","providerType":"anthropic","label":"x","apiKey":"k","models":[` + configuredAccountModel(t, "m") + `,` + configuredAccountModel(t, "m") + `]}`, webapi.ErrorCodeInvalidBody}, {`{"source":"custom","providerType":"anthropic","label":"x","apiKey":"two\nlines"}`, webapi.ErrorCodeInvalidBody}, {`{"source":"custom","providerType":"anthropic","label":"  ","apiKey":"k"}`, webapi.ErrorCodeInvalidBody}, {`{"providerType":"anthropic","label":"x","apiKey":"k"}`, webapi.ErrorCodeInvalidBody}} {
		accountRefusal(t, accountRequest(ctx, t, b, "POST", "/api/providers", &master, row.body), 400, row.code)
	}
	path := "/api/providers/" + string(created.ID)
	edited := accountRequest(ctx, t, b, "PATCH", path, &master, `{"label":"Personal","apiKey":"sk-ant-secret-2","baseUrl":null,"models":null}`)
	accountEqual(t, edited.Status, 200)
	if bytes.Contains(edited.Body, []byte("sk-ant-secret")) {
		t.Fatal("edited key disclosed")
	}
	entry := accountDecode(t, edited, webapi.DecodeProviderAnswer).Provider
	accountEqual(t, entry.Label, "Personal")
	if entry.BaseURL != nil || entry.Models != nil {
		t.Fatal(entry)
	}
	live := accountModels(ctx, t, b, &master, "")
	accountEqual(t, live.Providers[0].Models[0].ID, "claude-opus-4-8")
	body := `{"source":"custom","providerType":"anthropic","label":"Second","apiKey":"k"}`
	second := accountProviderEntry(ctx, t, b, &master, body)
	third := accountProviderEntry(ctx, t, b, &master, body)
	if second.ID == third.ID {
		t.Fatal("API key entries merged")
	}
	for _, extra := range []webapi.ProviderDTO{second, third} {
		accountRequest(ctx, t, b, "DELETE", "/api/providers/"+string(extra.ID), &master, "")
	}
	if err := h.AddUser(ctx, "alice@example.test", "alice-pass-1", webapi.RoleUser); err != nil {
		t.Fatal(err)
	}
	alice := accountLogin(ctx, t, b, "alice@example.test", "alice-pass-1")
	accountEqual(t, len(accountDecode(t, accountRequest(ctx, t, b, "GET", "/api/providers", &alice, ""), webapi.DecodeProviders).Providers), 0)
	for _, user := range []*backendtest.Session{&master, &alice} {
		imported := accountRequest(ctx, t, b, "POST", "/api/providers/setup-token", user, accountJSON(t, contract.Field{Name: "token", Value: "token-of-" + string(user.User.Email)}, contract.Field{Name: "label", Value: "Claude"}))
		accountEqual(t, imported.Status, 201)
		id := accountDecode(t, imported, webapi.DecodeProviderAnswer).Provider.ID
		accountEqual(t, accountRequest(ctx, t, b, "DELETE", "/api/providers/"+string(id), user, "").Status, 204)
	}
	accountRefusal(t, accountRequest(ctx, t, b, "DELETE", path, &alice, ""), 404, webapi.ErrorCodeProviderNotFound)
	accountRefusal(t, accountRequest(ctx, t, b, "GET", path+"/status", &alice, ""), 404, webapi.ErrorCodeProviderNotFound)
	accountEqual(t, accountRequest(ctx, t, b, "DELETE", path, &master, "").Status, 204)
	accountEqual(t, len(accountDecode(t, accountRequest(ctx, t, b, "GET", "/api/providers", &master, ""), webapi.DecodeProviders).Providers), 0)
	accountEqual(t, len(accountModels(ctx, t, b, &master, "").Providers), 0)
	accountRefusal(t, accountRequest(ctx, t, b, "PATCH", path, &master, `{"label":"Gone"}`), 404, webapi.ErrorCodeProviderNotFound)
}

func TestADirectoryCatalogIsCachedRefreshedOnDemandAndKeptAfterAFailedRefresh(t *testing.T) {
	ctx, cancel := context.WithTimeout(t.Context(), 10*time.Second)
	defer cancel()
	h := accountHarness(ctx, t)
	directory := &backendtest.AccountDirectory{}
	h.Config.Families.Register("scripted", &backendtest.AccountFamily{T: t, Directory: directory})
	b, master := accountStart(ctx, t, h)
	entry := accountProviderEntry(ctx, t, b, &master, `{"source":"custom","providerType":"scripted","label":"Scripted","apiKey":"k"}`)
	ids := func(c webapi.ModelCatalog) []string {
		var ids []string
		for _, m := range c.Providers[0].Models {
			ids = append(ids, m.ID)
		}
		return ids
	}
	directory.Answer(backendtest.AccountCatalog("a", "b"), nil)
	first := accountModels(ctx, t, b, &master, "")
	accountEqual(t, ids(first), []string{"a", "b"})
	accountEqual(t, first.Providers[0].Stale, false)
	accountEqual(t, directory.Reads(), 1)
	accountEqual(t, first.Providers[0].SourceFetchedAt, core.Timestamp("2026-09-24T07:00:00.000Z"))
	accountModels(ctx, t, b, &master, "")
	accountEqual(t, directory.Reads(), 1)
	for _, query := range []string{"?refresh=1", "?refresh=TRUE", "?refresh="} {
		accountRefusal(t, accountRequest(ctx, t, b, "GET", "/api/models"+query, &master, ""), 400, webapi.ErrorCodeInvalidQuery)
	}
	directory.Answer(backendtest.AccountCatalog("a", "b", "c"), nil)
	accountEqual(t, len(ids(accountModels(ctx, t, b, &master, "?refresh=true"))), 3)
	directory.Answer(core.ProviderModelList{}, &provider.CatalogError{Kind: provider.CatalogUnavailable, Message: "the directory is down"})
	failed := accountModels(ctx, t, b, &master, "?refresh=true")
	accountEqual(t, len(ids(failed)), 3)
	accountEqual(t, failed.Providers[0].Stale, true)
	accountEqual(t, failed.Providers[0].Warnings, []string{"the directory is down"})
	if err := b.Close(ctx); err != nil {
		t.Fatal(err)
	}
	var err error
	b, err = h.Start(ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	restored := accountModels(ctx, t, b, &master, "")
	accountEqual(t, len(ids(restored)), 3)
	accountEqual(t, directory.Reads(), 3)
	if err := h.Clock.Advance(16 * time.Minute); err != nil {
		t.Fatal(err)
	}
	directory.Answer(backendtest.AccountCatalog("d"), nil)
	expired := accountModels(ctx, t, b, &master, "")
	accountEqual(t, len(ids(expired)), 3)
	accountEqual(t, expired.Providers[0].Stale, true)
	refreshed := expired
	for len(ids(refreshed)) != 1 || ids(refreshed)[0] != "d" {
		refreshed = accountModels(ctx, t, b, &master, "")
	}
	accountEqual(t, ids(refreshed), []string{"d"})
	accountEqual(t, refreshed.Providers[0].Stale, false)
	accountEqual(t, directory.Reads(), 4)
	accountRequest(ctx, t, b, "PATCH", "/api/providers/"+string(entry.ID), &master, `{"models":[`+configuredAccountModel(t, "typed")+`]}`)
	typed := accountModels(ctx, t, b, &master, "?refresh=true")
	accountEqual(t, ids(typed), []string{"typed"})
	accountEqual(t, typed.Providers[0].Stale, false)
	accountEqual(t, directory.Reads(), 4)
	accountEqual(t, typed.Providers[0].SourceFetchedAt, core.Timestamp("1970-01-01T00:00:00.000Z"))
	model := typed.Providers[0].Models[0]
	accountEqual(t, *model.Selection.Model.OutputLimit, uint32(4000))
	accountEqual(t, model.ServiceTiers[0].ID, "priority")
	accountEqual(t, *model.AcceptedExtensions, []core.FileExtension{core.FileExtensionPDF})
}

type accountRefusingFamily struct{}

func (accountRefusingFamily) Credential() webapi.CredentialKind { return webapi.CredentialKindAPIKey }
func (accountRefusingFamily) Wires() []core.WireAPI             { return nil }
func (accountRefusingFamily) Provider(providers.FamilyArgs) (provider.Provider, error) {
	return nil, errors.New("the scripted family refuses")
}

func TestAnEntryWhoseProviderCannotBeBuiltAnswersItsStatusWithTheReason(t *testing.T) {
	ctx := t.Context()
	h := accountHarness(ctx, t)
	h.Config.Families.Register("refusing", accountRefusingFamily{})
	b, master := accountStart(ctx, t, h)
	entry := accountProviderEntry(ctx, t, b, &master, `{"source":"custom","providerType":"refusing","label":"Broken","apiKey":"key","models":[`+configuredAccountModel(t, "m")+`]}`)
	e := accountRefusal(t, accountRequest(ctx, t, b, "GET", "/api/providers/"+string(entry.ID)+"/status", &master, ""), 502, webapi.ErrorCodeProviderStatusFailed)
	accountEqual(t, e.Message, "the scripted family refuses")
}

// accountVendorResponse queues a literal vendor document without reordering its fields.
func accountVendorResponse(body string) providertest.MockResponse {
	return providertest.MockResponse{Status: 200, Chunks: [][]byte{[]byte(body)}}
}

// accountMessagesAnswer streams the Rust fixture's minimal Messages API answer.
func accountMessagesAnswer() providertest.MockResponse {
	return providertest.EventStream("event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-opus-4-8\",\"content\":[],\"usage\":{\"input_tokens\":3,\"output_tokens\":1}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n")
}

func TestTheProviderTestSendsOneRealRequestThroughTheEntrysFamily(t *testing.T) {
	ctx := t.Context()
	vendor := providertest.StartVendor(t)
	b, master := accountStart(ctx, t, accountHarness(ctx, t))
	entry := accountProviderEntry(ctx, t, b, &master, `{"source":"custom","providerType":"anthropic","label":"Work","apiKey":"sk-ant-test","baseUrl":"`+vendor.URL("/v1")+`"}`)
	path := "/api/providers/" + string(entry.ID) + "/test"
	vendor.Respond(accountMessagesAnswer())
	passed := accountDecode(t, accountRequest(ctx, t, b, "POST", path, &master, `{"modelId":"claude-opus-4-8"}`), webapi.DecodeTestResult)
	accountEqual[webapi.TestResult](t, passed, &webapi.TestResultPassed{Model: "Claude Opus 4.8"})
	sent := vendor.Requests()[0]
	accountEqual(t, sent.URI, "/v1/messages")
	accountEqual(t, sent.Header("x-api-key"), "sk-ant-test")
	body, ok := sent.JSON(t).(map[string]any)
	if !ok {
		t.Fatal("vendor request is not an object")
	}
	accountEqual(t, body["system"], any([]any{map[string]any{"type": "text", "text": "Reply with the word ok."}}))
	if bytes.Contains(sent.Body, []byte("cache_control")) {
		t.Fatal("connection test adds cache markers")
	}
	vendor.Respond(providertest.MockResponse{Status: 401, Chunks: [][]byte{[]byte(`{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}`)}})
	result := accountDecode(t, accountRequest(ctx, t, b, "POST", path, &master, `{"modelId":"claude-opus-4-8"}`), webapi.DecodeTestResult)
	failed, ok := result.(*webapi.TestResultFailed)
	if !ok {
		t.Fatalf("result: %#v", result)
	}
	if !strings.Contains(failed.Message, "HTTP 401") || !strings.Contains(failed.Message, "invalid x-api-key") {
		t.Fatal(failed.Message)
	}
	accountEqual(t, *failed.Model, "Claude Opus 4.8")
	accountEqual[webapi.TestResult](t, accountDecode(t, accountRequest(ctx, t, b, "POST", path, &master, `{"modelId":"no-such-model"}`), webapi.DecodeTestResult), &webapi.TestResultFailed{Message: "This provider lists no model no-such-model"})
	accountRefusal(t, accountRequest(ctx, t, b, "POST", path, &master, `{"modelId":"claude-opus-4-8","credentialId":"cred-1"}`), 404, webapi.ErrorCodeAccountNotFound)
	accountEqual(t, len(vendor.Requests()), 2)
	for _, row := range []struct{ family, wire, stream, path, header, value string }{{"openai", "chat-completions", "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n", "/v1/chat/completions", "authorization", "Bearer sk-2"}, {"openai", "", "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n", "/v1/responses", "", ""}, {"google", "", "", "/v1/models/m:streamGenerateContent?alt=sse", "x-goog-api-key", "sk-2"}} {
		wire := ""
		if row.wire != "" {
			wire = `,"wireApi":"` + row.wire + `"`
		}
		entry := accountProviderEntry(ctx, t, b, &master, `{"source":"custom","providerType":"`+row.family+`","label":"`+row.family+`","apiKey":"sk-2","baseUrl":"`+vendor.URL("/v1")+`","models":[`+configuredAccountModel(t, "m")+`]`+wire+`}`)
		vendor.Respond(providertest.EventStream(row.stream))
		accountEqual[webapi.TestResult](t, accountDecode(t, accountRequest(ctx, t, b, "POST", "/api/providers/"+string(entry.ID)+"/test", &master, `{"modelId":"m"}`), webapi.DecodeTestResult), &webapi.TestResultPassed{Model: "m display"})
		requests := vendor.Requests()
		sent := requests[len(requests)-1]
		parsed, err := url.Parse(sent.URI)
		if err != nil {
			t.Fatal(err)
		}
		want, err := url.Parse(row.path)
		if err != nil {
			t.Fatal(err)
		}
		accountEqual(t, parsed.Path, want.Path)
		if row.header != "" {
			accountEqual(t, sent.Header(row.header), row.value)
		}
	}
}

func TestOnASharedInstanceOnlyTheMasterConfiguresAndEveryoneInfersWithItsEntries(t *testing.T) {
	ctx := t.Context()
	h := accountHarness(ctx, t)
	h.Config.Families.Register("scripted", &backendtest.AccountFamily{T: t, Directory: &backendtest.AccountDirectory{}})
	b, master := accountStart(ctx, t, h)
	if err := h.AddUser(ctx, "admin@example.test", "admin-pass-1", webapi.RoleAdmin); err != nil {
		t.Fatal(err)
	}
	if err := h.AddUser(ctx, "bob@example.test", "bob-pass-1", webapi.RoleUser); err != nil {
		t.Fatal(err)
	}
	admin := accountLogin(ctx, t, b, "admin@example.test", "admin-pass-1")
	bob := accountLogin(ctx, t, b, "bob@example.test", "bob-pass-1")
	body := `{"source":"custom","providerType":"scripted","label":"Instance","apiKey":"k","models":[` + configuredAccountModel(t, "m") + `]}`
	for _, user := range []*backendtest.Session{&admin, &bob} {
		accountRefusal(t, accountRequest(ctx, t, b, "POST", "/api/providers", user, body), 403, webapi.ErrorCodeForbidden)
	}
	shared := accountProviderEntry(ctx, t, b, &master, body)
	accountEqual(t, accountDecode(t, accountRequest(ctx, t, b, "GET", "/api/providers", &bob, ""), webapi.DecodeProviders).Providers, []webapi.ProviderDTO{shared})
	models := accountModels(ctx, t, b, &bob, "")
	accountEqual(t, models.Providers[0].ProviderID, shared.ID)
	accountEqual[webapi.Availability](t, models.Providers[0].Availability, &webapi.AvailabilityAvailable{})
	path := "/api/providers/" + string(shared.ID)
	accountRefusal(t, accountRequest(ctx, t, b, "DELETE", path, &bob, ""), 403, webapi.ErrorCodeForbidden)
	accountRefusal(t, accountRequest(ctx, t, b, "POST", path+"/test", &bob, `{"modelId":"m"}`), 403, webapi.ErrorCodeForbidden)
	const id = "3c1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a01"
	accountEqual(t, accountRequest(ctx, t, b, "POST", "/api/conversations", &bob, `{"id":"`+id+`"}`).Status, 201)
	selection := `{"model":{"providerId":"` + string(shared.ID) + `","modelId":"m"}}`
	accountEqual(t, accountRequest(ctx, t, b, "PATCH", "/api/conversations/"+id, &bob, selection).Status, 200)
	socket, err := backendtest.OpenAccountSocket(ctx, t, b, &bob, id)
	if err != nil {
		t.Fatal(err)
	}
	if err := socket.Open(ctx); err != nil {
		t.Fatal(err)
	}
	turn, err := socket.Chat(ctx, "m1", "hello")
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := contract.EncodeJSON(turn)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(encoded, []byte(`"ok"`)) {
		t.Fatal(string(encoded))
	}
	// The scenario has finished reading this socket; teardown may find it already closed.
	_ = socket.Conn.CloseNow()
	if err := b.Close(ctx); err != nil {
		t.Fatal(err)
	}
	b, err = h.StartInMode(ctx, t, webapi.InstanceModeIsolated)
	if err != nil {
		t.Fatal(err)
	}
	master = accountLogin(ctx, t, b, backendtest.MasterEmail, backendtest.MasterPassword)
	bob = accountLogin(ctx, t, b, "bob@example.test", "bob-pass-1")
	socket, err = backendtest.OpenAccountSocket(ctx, t, b, &bob, id)
	if err != nil {
		t.Fatal(err)
	}
	if err := socket.Send(ctx, &framewire.OpenFrame{}); err != nil {
		t.Fatal(err)
	}
	frame, err := socket.Next(ctx)
	if err != nil {
		t.Fatal(err)
	}
	refused, ok := frame.(*framewire.ErrorFrame)
	if !ok {
		t.Fatalf("frame: %#v", frame)
	}
	accountEqual(t, *refused.Code, "provider_not_found")
	accountRefusal(t, accountRequest(ctx, t, b, "PATCH", "/api/conversations/"+id, &bob, selection), 404, webapi.ErrorCodeProviderNotFound)
	// The scenario has finished reading this socket; teardown may find it already closed.
	_ = socket.Conn.CloseNow()
	accountEqual(t, len(accountDecode(t, accountRequest(ctx, t, b, "GET", "/api/providers", &master, ""), webapi.DecodeProviders).Providers), 1)
	accountEqual(t, len(accountDecode(t, accountRequest(ctx, t, b, "GET", "/api/providers", &bob, ""), webapi.DecodeProviders).Providers), 0)
}

func TestAVendorEntryTakesItsFamilyWireAndEndpointFromModelsDevAndReadsItsLiveModels(t *testing.T) {
	ctx := t.Context()
	vendor := providertest.StartVendor(t)
	h := accountHarness(ctx, t)
	directory := &backendtest.AccountDirectory{}
	h.Config.Families.Register("openai", &backendtest.AccountFamily{T: t, Directory: directory, WireTypes: []core.WireAPI{core.WireAPIResponses, core.WireAPIChatCompletions}})
	var err error
	h.Config.ModelsDevURL, err = url.Parse(vendor.URL("/api.json"))
	if err != nil {
		t.Fatal(err)
	}
	b, master := accountStart(ctx, t, h)
	document := `{"deepseek":{"id":"deepseek","name":"DeepSeek","npm":"@ai-sdk/openai-compatible","api":"https://api.deepseek.com","doc":"https://api-docs.deepseek.com","models":{"deepseek-v4":{"name":"DeepSeek V4","reasoning":true,"tool_call":true,"attachment":false,"limit":{"context":128000,"output":32000},"cost":{"input":0.3,"output":1.2,"cache_read":0.03,"cache_write":0}},"deepseek-v4-flash":{"name":"DeepSeek V4 Flash","limit":{"context":128000}}}},"minimax":{"id":"minimax","name":"MiniMax","npm":"@ai-sdk/anthropic","api":"https://api.minimax.io/anthropic/v1","models":{"minimax-m3":{"name":"MiniMax M3","limit":{"context":200000,"output":64000}}}},"zai":{"id":"zai","name":"z.ai","npm":"@ai-sdk/google","models":{}},"amazon-bedrock":{"id":"amazon-bedrock","name":"Amazon Bedrock","npm":"@ai-sdk/amazon-bedrock","models":{"anthropic.claude-sonnet":{"name":"Claude Sonnet on Bedrock"}}},"github-copilot":{"id":"github-copilot","name":"GitHub Copilot","npm":"@ai-sdk/openai-compatible","api":"https://api.githubcopilot.com","models":{"gpt-5.5":{"name":"GPT-5.5"}}}}`
	response := accountVendorResponse(document)
	response.Headers = http.Header{"Etag": []string{`"fixture"`}}
	vendor.Respond(response)
	offered := accountDecode(t, accountRequest(ctx, t, b, "GET", "/api/providers/catalog", &master, ""), webapi.DecodeVendorCatalog)
	expected := accountDecode(t, backendtest.Answer{Body: []byte(`{"vendors":[{"id":"deepseek","name":"DeepSeek","providerType":"openai","wireApi":"chat-completions","baseUrl":"https://api.deepseek.com","doc":"https://api-docs.deepseek.com"},{"id":"minimax","name":"MiniMax","providerType":"anthropic","baseUrl":"https://api.minimax.io/anthropic/v1","doc":null},{"id":"zai","name":"z.ai","providerType":"google","baseUrl":null,"doc":null}],"subscriptions":[]}`)}, webapi.DecodeVendorCatalog)
	accountEqual(t, offered.Vendors, expected.Vendors)
	accountRefusal(t, accountRequest(ctx, t, b, "POST", "/api/providers", &master, `{"source":"vendor","vendorId":"amazon-bedrock","label":"x","apiKey":"k"}`), 400, webapi.ErrorCodeUnknownVendor)
	deepseek := accountProviderEntry(ctx, t, b, &master, `{"source":"vendor","vendorId":"deepseek","label":"DeepSeek","apiKey":"sk-1"}`)
	accountEqual(t, deepseek.ProviderType, "openai")
	accountEqual(t, *deepseek.WireAPI, core.WireAPIChatCompletions)
	accountEqual(t, *deepseek.VendorID, "deepseek")
	accountEqual(t, string(*deepseek.BaseURL), "https://api.deepseek.com/")
	vendor.Respond(providertest.MockResponse{Status: 304})
	live := accountModels(ctx, t, b, &master, "")
	accountEqual(t, vendor.Requests()[1].Header("if-none-match"), `"fixture"`)
	var ids []string
	for _, m := range live.Providers[0].Models {
		ids = append(ids, m.ID)
	}
	accountEqual(t, ids, []string{"deepseek-v4", "deepseek-v4-flash"})
	v4 := live.Providers[0].Models[0]
	accountEqual(t, *v4.ContextWindow, uint32(128000))
	accountEqual(t, *v4.OutputLimit, uint32(32000))
	accountEqual(t, *v4.SupportsReasoning, true)
	accountEqual(t, string(v4.Selection.ProviderID), string(deepseek.ID))
	accountEqual(t, v4.Selection.Model.ContextWindow, uint32(128000))
	accountEqual(t, live.Providers[0].Stale, false)
	accountEqual(t, directory.Reads(), 0)
	path := "/api/providers/" + string(deepseek.ID)
	accountRequest(ctx, t, b, "PATCH", path, &master, `{"models":[`+configuredAccountModel(t, "deepseek-v4")+`]}`)
	typed := accountModels(ctx, t, b, &master, "?refresh=true")
	accountEqual(t, len(typed.Providers[0].Models), 1)
	accountEqual(t, len(vendor.Requests()), 2)
	accountRequest(ctx, t, b, "PATCH", path, &master, `{"models":null}`)
	vendor.Respond(providertest.MockResponse{Status: 304})
	accountEqual(t, len(accountModels(ctx, t, b, &master, "").Providers[0].Models), 2)
}

func TestAnAnthropicCompatibleVendorReceivesAnEffortAsATokenBudget(t *testing.T) {
	ctx := t.Context()
	vendor := providertest.StartVendor(t)
	vendor.RespondAt("/api.json", accountVendorResponse(`{"moonshot":{"id":"moonshot","name":"Moonshot","npm":"@ai-sdk/anthropic","api":"https://api.moonshot.example/anthropic","models":{}}}`))
	h := accountHarness(ctx, t)
	var err error
	h.Config.ModelsDevURL, err = url.Parse(vendor.URL("/api.json"))
	if err != nil {
		t.Fatal(err)
	}
	b, master := accountStart(ctx, t, h)
	model := `{"id":"m","displayName":"M","contextWindow":100000,"outputLimit":64000,"thinkingEfforts":["low","high"],"acceptedExtensions":null,"fastTier":null}`
	compatible := accountProviderEntry(ctx, t, b, &master, `{"source":"vendor","vendorId":"moonshot","label":"Moonshot","apiKey":"key","baseUrl":"`+vendor.URL("/v1")+`","models":[`+model+`]}`)
	anthropic := accountProviderEntry(ctx, t, b, &master, `{"source":"custom","providerType":"anthropic","label":"Anthropic","apiKey":"key","baseUrl":"`+vendor.URL("/v1")+`","models":[`+model+`]}`)
	const id = "3c1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a01"
	accountEqual(t, accountRequest(ctx, t, b, "POST", "/api/conversations", &master, `{"id":"`+id+`"}`).Status, 201)
	var sent []map[string]any
	for i, entry := range []webapi.ProviderDTO{compatible, anthropic} {
		accountEqual(t, accountRequest(ctx, t, b, "PATCH", "/api/conversations/"+id, &master, `{"model":{"providerId":"`+string(entry.ID)+`","modelId":"m"},"thinkingEffort":"high"}`).Status, 200)
		socket, err := backendtest.OpenAccountSocket(ctx, t, b, &master, id)
		if err != nil {
			t.Fatal(err)
		}
		if err := socket.Open(ctx); err != nil {
			t.Fatal(err)
		}
		vendor.Respond(providertest.EventStream(`event: message_start
data: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-opus-4-8","content":[],"usage":{"input_tokens":1,"output_tokens":0}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Done."}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}

event: message_stop
data: {"type":"message_stop"}

`))
		if _, err := socket.Chat(ctx, fmt.Sprintf("m-%d", i), "Think hard."); err != nil {
			t.Fatal(err)
		}
		requests := vendor.Requests()
		for j := len(requests) - 1; j >= 0; j-- {
			if requests[j].URI == "/v1/messages" {
				body, ok := requests[j].JSON(t).(map[string]any)
				if !ok {
					t.Fatal("vendor request is not an object")
				}
				sent = append(sent, body)
				break
			}
		}
		// The scenario has finished reading this socket; teardown may find it already closed.
		_ = socket.Conn.CloseNow()
	}
	accountEqual(t, len(sent), 2)
	accountEqual(t, sent[0]["thinking"], any(map[string]any{"type": "enabled", "budget_tokens": float64(32768)}))
	if _, ok := sent[0]["output_config"]; ok {
		t.Fatal(sent[0])
	}
	thinking, ok := sent[1]["thinking"].(map[string]any)
	if !ok {
		t.Fatal("Anthropic request has no thinking object")
	}
	accountEqual(t, thinking["type"], any("adaptive"))
	accountEqual(t, sent[1]["output_config"], any(map[string]any{"effort": "high"}))
}
