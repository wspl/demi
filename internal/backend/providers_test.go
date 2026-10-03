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
	return accountJSON(
		t,
		contract.Field{Name: "id", Value: id},
		contract.Field{Name: "displayName", Value: id + " display"},
		contract.Field{Name: "contextWindow", Value: 64000},
		contract.Field{Name: "outputLimit", Value: 4000},
		contract.Field{Name: "thinkingEfforts", Value: []string{"low", "high"}},
		contract.Field{Name: "acceptedExtensions", Value: []string{"pdf"}},
		contract.Field{Name: "fastTier", Value: "priority"},
	)
}

// accountProviderEntry creates a provider through its HTTP boundary.
func accountProviderEntry(
	ctx context.Context,
	t *testing.T,
	server *backendtest.TestBackend,
	s *backendtest.Session,
	body string,
) webapi.ProviderDTO {
	t.Helper()
	a := conversationRequest(ctx, t, server, s, "POST", "/api/providers", body, 201)
	return conversationDecode(t, a, webapi.DecodeProviderAnswer).Provider
}

// accountModels reads the catalog through the same route as a model picker.
func accountModels(
	ctx context.Context,
	t *testing.T,
	server *backendtest.TestBackend,
	s *backendtest.Session,
	query string,
) webapi.ModelCatalog {
	t.Helper()
	a := conversationRequest(ctx, t, server, s, "GET", "/api/models"+query, "", 200)
	return conversationDecode(t, a, webapi.DecodeModelCatalog)
}

// TestAnAPIKeyEntryIsSealedAtRestAndAnsweredWithoutItsKey
// checks API key storage and public provider answers.
func TestAnAPIKeyEntryIsSealedAtRestAndAnsweredWithoutItsKey(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	harness.Config.Mode = webapi.InstanceModeIsolated
	harness.Config.Families, _ = backendtest.AccountFamilies(t, nil)
	harness.Config.Families.Register(
		"scripted",
		&backendtest.AccountFamily{T: t, Directory: &backendtest.AccountDirectory{}},
	)
	server, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	created := accountProviderEntry(
		ctx,
		t,
		server,
		&master,
		`{"source":"custom","providerType":"anthropic","label":" Work ",`+
			`"apiKey":"sk-ant-secret-1","baseUrl":"https://proxy.example/v1","models":[`+configuredAccountModel(
			t,
			"claude-work",
		)+`]}`,
	)
	conversationEqual(t, created.Kind, webapi.CredentialKindAPIKey)
	conversationEqual(t, created.ProviderType, "anthropic")
	conversationEqual(t, created.Label, "Work")
	conversationEqual(t, string(*created.BaseURL), "https://proxy.example/v1")
	if created.WireAPI != nil || created.VendorID != nil {
		t.Fatal(created)
	}
	conversationEqual(t, (*created.Models)[0].ID, "claude-work")
	listed := conversationRequest(ctx, t, server, &master, "GET", "/api/providers", "", 200)
	conversationEqual(t, conversationDecode(t, listed, webapi.DecodeProviders).Providers, []webapi.ProviderDTO{created})
	if bytes.Contains(listed.Body, []byte("sk-ant-secret-1")) {
		t.Fatal("key disclosed")
	}
	db, err := harness.ControlDatabase(ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	var sealed []byte
	if err := db.QueryRowContext(ctx, "SELECT config FROM providers WHERE id = ?", created.ID).
		Scan(&sealed); err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(sealed, []byte("sk-ant-secret-1")) {
		t.Fatal("key not sealed")
	}
	for _, row := range []struct {
		body string
		code webapi.ErrorCode
	}{
		{`{"source":"custom","providerType":"nope","label":"x","apiKey":"k"}`, webapi.ErrorCodeUnknownProviderType},
		{
			`{"source":"custom","providerType":"anthropic","wireApi":"responses","label":"x","apiKey":"k"}`,
			webapi.ErrorCodeInvalidBody,
		},
		{
			`{"source":"custom","providerType":"anthropic","label":"x","apiKey":"k","models":[` +
				configuredAccountModel(t, "m") + `,` + configuredAccountModel(t, "m") + `]}`,
			webapi.ErrorCodeInvalidBody,
		},
		{`{"source":"custom","providerType":"anthropic","label":"x","apiKey":"two\nlines"}`, webapi.ErrorCodeInvalidBody},
		{`{"source":"custom","providerType":"anthropic","label":"  ","apiKey":"k"}`, webapi.ErrorCodeInvalidBody},
		{`{"providerType":"anthropic","label":"x","apiKey":"k"}`, webapi.ErrorCodeInvalidBody},
	} {
		conversationRefusal(
			t,
			conversationRequest(ctx, t, server, &master, "POST", "/api/providers", row.body, 400),
			row.code,
		)
	}
	path := "/api/providers/" + string(created.ID)
	edited := conversationRequest(
		ctx,
		t,
		server,
		&master,
		"PATCH",
		path,
		`{"label":"Personal","apiKey":"sk-ant-secret-2","baseUrl":null,"models":null}`,
		200,
	)
	if bytes.Contains(edited.Body, []byte("sk-ant-secret")) {
		t.Fatal("edited key disclosed")
	}
	entry := conversationDecode(t, edited, webapi.DecodeProviderAnswer).Provider
	conversationEqual(t, entry.Label, "Personal")
	if entry.BaseURL != nil || entry.Models != nil {
		t.Fatal(entry)
	}
	live := accountModels(ctx, t, server, &master, "")
	conversationEqual(t, live.Providers[0].Models[0].ID, "claude-opus-4-8")
	body := `{"source":"custom","providerType":"anthropic","label":"Second","apiKey":"k"}`
	second := accountProviderEntry(ctx, t, server, &master, body)
	third := accountProviderEntry(ctx, t, server, &master, body)
	if second.ID == third.ID {
		t.Fatal("API key entries merged")
	}
	for _, extra := range []webapi.ProviderDTO{second, third} {
		conversationRequest(ctx, t, server, &master, "DELETE", "/api/providers/"+string(extra.ID), "", 204)
	}
	if err := harness.AddUser(ctx, "alice@example.test", "alice-pass-1", webapi.RoleUser); err != nil {
		t.Fatal(err)
	}
	alice, err := server.Login(ctx, "alice@example.test", "alice-pass-1")
	wireMust(t, err)
	conversationEqual(
		t,
		len(
			conversationDecode(
				t,
				conversationRequest(ctx, t, server, &alice, "GET", "/api/providers", "", 200),
				webapi.DecodeProviders,
			).Providers,
		),
		0,
	)
	for _, user := range []*backendtest.Session{&master, &alice} {
		imported := conversationRequest(
			ctx,
			t,
			server,
			user,
			"POST",
			"/api/providers/setup-token",
			accountJSON(
				t,
				contract.Field{Name: "token", Value: "token-of-" + string(user.User.Email)},
				contract.Field{Name: "label", Value: "Claude"},
			),
			201,
		)
		id := conversationDecode(t, imported, webapi.DecodeProviderAnswer).Provider.ID
		conversationRequest(ctx, t, server, user, "DELETE", "/api/providers/"+string(id), "", 204)
	}
	conversationRefusal(
		t,
		conversationRequest(ctx, t, server, &alice, "DELETE", path, "", 404),
		webapi.ErrorCodeProviderNotFound,
	)
	conversationRefusal(
		t,
		conversationRequest(ctx, t, server, &alice, "GET", path+"/status", "", 404),
		webapi.ErrorCodeProviderNotFound,
	)
	conversationRequest(ctx, t, server, &master, "DELETE", path, "", 204)
	conversationEqual(
		t,
		len(
			conversationDecode(
				t,
				conversationRequest(ctx, t, server, &master, "GET", "/api/providers", "", 200),
				webapi.DecodeProviders,
			).Providers,
		),
		0,
	)
	conversationEqual(t, len(accountModels(ctx, t, server, &master, "").Providers), 0)
	conversationRefusal(
		t,
		conversationRequest(ctx, t, server, &master, "PATCH", path, `{"label":"Gone"}`, 404),
		webapi.ErrorCodeProviderNotFound,
	)
}

// TestADirectoryCatalogIsCachedRefreshedOnDemandAndKeptAfterAFailedRefresh
// checks model catalog caching and refresh failures.
func TestADirectoryCatalogIsCachedRefreshedOnDemandAndKeptAfterAFailedRefresh(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	directory := &backendtest.AccountDirectory{}
	harness.Config.Families.Register("scripted", &backendtest.AccountFamily{T: t, Directory: directory})
	server, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	entry := accountProviderEntry(
		ctx,
		t,
		server,
		&master,
		`{"source":"custom","providerType":"scripted","label":"Scripted","apiKey":"k"}`,
	)
	ids := func(c webapi.ModelCatalog) []string {
		var ids []string
		for _, m := range c.Providers[0].Models {
			ids = append(ids, m.ID)
		}
		return ids
	}
	directory.Answer(backendtest.AccountCatalog("a", "b"), nil)
	first := accountModels(ctx, t, server, &master, "")
	conversationEqual(t, ids(first), []string{"a", "b"})
	conversationEqual(t, first.Providers[0].Stale, false)
	conversationEqual(t, directory.Reads(), 1)
	conversationEqual(t, first.Providers[0].SourceFetchedAt, core.Timestamp("2026-09-24T07:00:00.000Z"))
	accountModels(ctx, t, server, &master, "")
	conversationEqual(t, directory.Reads(), 1)
	for _, query := range []string{"?refresh=1", "?refresh=TRUE", "?refresh="} {
		conversationRefusal(
			t,
			conversationRequest(ctx, t, server, &master, "GET", "/api/models"+query, "", 400),
			webapi.ErrorCodeInvalidQuery,
		)
	}
	directory.Answer(backendtest.AccountCatalog("a", "b", "c"), nil)
	conversationEqual(t, len(ids(accountModels(ctx, t, server, &master, "?refresh=true"))), 3)
	directory.Answer(
		core.ProviderModelList{},
		&provider.CatalogError{Kind: provider.CatalogUnavailable, Message: "the directory is down"},
	)
	failed := accountModels(ctx, t, server, &master, "?refresh=true")
	conversationEqual(t, len(ids(failed)), 3)
	conversationEqual(t, failed.Providers[0].Stale, true)
	conversationEqual(t, failed.Providers[0].Warnings, []string{"the directory is down"})
	if err := server.Close(ctx); err != nil {
		t.Fatal(err)
	}
	server, err = harness.Start(ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	restored := accountModels(ctx, t, server, &master, "")
	conversationEqual(t, len(ids(restored)), 3)
	conversationEqual(t, directory.Reads(), 3)
	if err := harness.Clock.Advance(16 * time.Minute); err != nil {
		t.Fatal(err)
	}
	directory.Answer(backendtest.AccountCatalog("d"), nil)
	expired := accountModels(ctx, t, server, &master, "")
	conversationEqual(t, len(ids(expired)), 3)
	conversationEqual(t, expired.Providers[0].Stale, true)
	// Bound the refresh wait, not the setup and backend restart preceding it.
	refreshCtx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	refreshed := expired
	for len(ids(refreshed)) != 1 || ids(refreshed)[0] != "d" {
		refreshed = accountModels(refreshCtx, t, server, &master, "")
	}
	conversationEqual(t, ids(refreshed), []string{"d"})
	conversationEqual(t, refreshed.Providers[0].Stale, false)
	conversationEqual(t, directory.Reads(), 4)
	conversationRequest(
		ctx,
		t,
		server,
		&master,
		"PATCH",
		"/api/providers/"+string(entry.ID),
		`{"models":[`+configuredAccountModel(t, "typed")+`]}`,
		200,
	)
	typed := accountModels(ctx, t, server, &master, "?refresh=true")
	conversationEqual(t, ids(typed), []string{"typed"})
	conversationEqual(t, typed.Providers[0].Stale, false)
	conversationEqual(t, directory.Reads(), 4)
	conversationEqual(t, typed.Providers[0].SourceFetchedAt, core.Timestamp("1970-01-01T00:00:00.000Z"))
	model := typed.Providers[0].Models[0]
	conversationEqual(t, *model.Selection.Model.OutputLimit, uint32(4000))
	conversationEqual(t, model.ServiceTiers[0].ID, "priority")
	conversationEqual(t, *model.AcceptedExtensions, []core.FileExtension{core.FileExtensionPDF})
}

type accountRefusingFamily struct{}

// Credential advertises an API key for the intentionally unbuildable family.
func (accountRefusingFamily) Credential() webapi.CredentialKind { return webapi.CredentialKindAPIKey }

// Wires advertises no wire APIs for the intentionally unbuildable family.
func (accountRefusingFamily) Wires() []core.WireAPI { return nil }

// Provider returns the scripted build failure used to observe provider status.
func (accountRefusingFamily) Provider(providers.FamilyArgs) (provider.Provider, error) {
	return nil, errors.New("the scripted family refuses")
}

// TestAnEntryWhoseProviderCannotBeBuiltAnswersItsStatusWithTheReason
// checks status reporting for a provider build failure.
func TestAnEntryWhoseProviderCannotBeBuiltAnswersItsStatusWithTheReason(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	harness.Config.Families.Register("refusing", accountRefusingFamily{})
	server, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	entry := accountProviderEntry(
		ctx,
		t,
		server,
		&master,
		`{"source":"custom","providerType":"refusing","label":"Broken","apiKey":"key","models":[`+configuredAccountModel(
			t,
			"m",
		)+`]}`,
	)
	eAnswer := conversationRequest(
		ctx,
		t,
		server,
		&master,
		"GET",
		"/api/providers/"+string(entry.ID)+"/status",
		"",
		502,
	)
	e := conversationDecode(t, eAnswer, webapi.DecodeErrorBody)
	conversationEqual(t, e.Code, webapi.ErrorCodeProviderStatusFailed)
	conversationEqual(t, e.Message, "the scripted family refuses")
}

// accountVendorResponse queues a literal vendor document without reordering its fields.
func accountVendorResponse(body string) providertest.MockResponse {
	return providertest.MockResponse{Status: 200, Chunks: [][]byte{[]byte(body)}}
}

// TestTheProviderTestSendsOneRealRequestThroughTheEntrysFamily
// checks provider test requests at the vendor boundary.
func TestTheProviderTestSendsOneRealRequestThroughTheEntrysFamily(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	vendor := providertest.StartVendor(t)
	server, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	id := conversationAnthropic(ctx, t, server, &master, vendor)
	path := "/api/providers/" + id + "/test"
	vendor.Respond(
		providertest.EventStream(
			conversationMessageStart(
				`{"input_tokens":3,"output_tokens":1}`,
			) + conversationTextBlock(
				t,
				0,
				"ok",
			) + anthropicEvent(
				"message_stop",
				`{"type":"message_stop"}`,
			),
		),
	)
	passed := conversationDecode(
		t,
		conversationRequest(ctx, t, server, &master, "POST", path, `{"modelId":"claude-opus-4-8"}`, 200),
		webapi.DecodeTestResult,
	)
	conversationEqual[webapi.TestResult](t, passed, &webapi.TestResultPassed{Model: "Claude Opus 4.8"})
	sent := vendor.Requests()[0]
	conversationEqual(t, sent.URI, "/v1/messages")
	conversationEqual(t, sent.Header("x-api-key"), "sk-ant-test")
	body, ok := sent.JSON(t).(map[string]any)
	if !ok {
		t.Fatal("vendor request is not an object")
	}
	conversationEqual(t, body["system"], any([]any{map[string]any{"type": "text", "text": "Reply with the word ok."}}))
	if bytes.Contains(sent.Body, []byte("cache_control")) {
		t.Fatal("connection test adds cache markers")
	}
	vendor.Respond(
		providertest.MockResponse{
			Status: 401,
			Chunks: [][]byte{
				[]byte(`{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}`),
			},
		},
	)
	result := conversationDecode(
		t,
		conversationRequest(ctx, t, server, &master, "POST", path, `{"modelId":"claude-opus-4-8"}`, 200),
		webapi.DecodeTestResult,
	)
	failed, ok := result.(*webapi.TestResultFailed)
	if !ok {
		t.Fatalf("result: %#v", result)
	}
	if !strings.Contains(failed.Message, "HTTP 401") || !strings.Contains(failed.Message, "invalid x-api-key") {
		t.Fatal(failed.Message)
	}
	conversationEqual(t, *failed.Model, "Claude Opus 4.8")
	conversationEqual[webapi.TestResult](
		t,
		conversationDecode(
			t,
			conversationRequest(ctx, t, server, &master, "POST", path, `{"modelId":"no-such-model"}`, 200),
			webapi.DecodeTestResult,
		),
		&webapi.TestResultFailed{Message: "This provider lists no model no-such-model"},
	)
	conversationRefusal(
		t,
		conversationRequest(
			ctx,
			t,
			server,
			&master,
			"POST",
			path,
			`{"modelId":"claude-opus-4-8","credentialId":"cred-1"}`,
			404,
		),
		webapi.ErrorCodeAccountNotFound,
	)
	conversationEqual(t, len(vendor.Requests()), 2)
	for _, row := range []struct{ family, wire, stream, path, header, value string }{
		{
			"openai",
			"chat-completions",
			"data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n",
			"/v1/chat/completions",
			"authorization",
			"Bearer sk-2",
		},
		{
			"openai",
			"",
			"data: {\"type\":\"response.completed\",\"response\":{" +
				"\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n",
			"/v1/responses",
			"",
			"",
		},
		{"google", "", "", "/v1/models/m:streamGenerateContent?alt=sse", "x-goog-api-key", "sk-2"},
	} {
		wire := ""
		if row.wire != "" {
			wire = `,"wireApi":"` + row.wire + `"`
		}
		entry := accountProviderEntry(
			ctx,
			t,
			server,
			&master,
			`{"source":"custom","providerType":"`+row.family+`","label":"`+row.family+`","apiKey":"sk-2","baseUrl":"`+vendor.URL(
				"/v1",
			)+`","models":[`+configuredAccountModel(
				t,
				"m",
			)+`]`+wire+`}`,
		)
		vendor.Respond(providertest.EventStream(row.stream))
		conversationEqual[webapi.TestResult](
			t,
			conversationDecode(
				t,
				conversationRequest(
					ctx,
					t,
					server,
					&master,
					"POST",
					"/api/providers/"+string(entry.ID)+"/test",
					`{"modelId":"m"}`,
					200,
				),
				webapi.DecodeTestResult,
			),
			&webapi.TestResultPassed{Model: "m display"},
		)
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
		conversationEqual(t, parsed.Path, want.Path)
		if row.header != "" {
			conversationEqual(t, sent.Header(row.header), row.value)
		}
	}
}

// TestOnASharedInstanceOnlyTheMasterConfiguresAndEveryoneInfersWithItsEntries
// checks shared provider permissions and inference.
func TestOnASharedInstanceOnlyTheMasterConfiguresAndEveryoneInfersWithItsEntries(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	harness.Config.Families.Register(
		"scripted",
		&backendtest.AccountFamily{T: t, Directory: &backendtest.AccountDirectory{}},
	)
	server, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	if err := harness.AddUser(ctx, "admin@example.test", "admin-pass-1", webapi.RoleAdmin); err != nil {
		t.Fatal(err)
	}
	if err := harness.AddUser(ctx, "bob@example.test", "bob-pass-1", webapi.RoleUser); err != nil {
		t.Fatal(err)
	}
	admin, err := server.Login(ctx, "admin@example.test", "admin-pass-1")
	wireMust(t, err)
	bob, err := server.Login(ctx, "bob@example.test", "bob-pass-1")
	wireMust(t, err)
	body := `{"source":"custom","providerType":"scripted","label":"Instance","apiKey":"k","models":[` +
		configuredAccountModel(
			t,
			"m",
		) + `]}`
	for _, user := range []*backendtest.Session{&admin, &bob} {
		conversationRefusal(
			t,
			conversationRequest(ctx, t, server, user, "POST", "/api/providers", body, 403),
			webapi.ErrorCodeForbidden,
		)
	}
	shared := accountProviderEntry(ctx, t, server, &master, body)
	conversationEqual(
		t,
		conversationDecode(
			t,
			conversationRequest(ctx, t, server, &bob, "GET", "/api/providers", "", 200),
			webapi.DecodeProviders,
		).Providers,
		[]webapi.ProviderDTO{shared},
	)
	models := accountModels(ctx, t, server, &bob, "")
	conversationEqual(t, models.Providers[0].ProviderID, shared.ID)
	conversationEqual[webapi.Availability](t, models.Providers[0].Availability, &webapi.AvailabilityAvailable{})
	path := "/api/providers/" + string(shared.ID)
	conversationRefusal(
		t,
		conversationRequest(ctx, t, server, &bob, "DELETE", path, "", 403),
		webapi.ErrorCodeForbidden,
	)
	conversationRefusal(
		t,
		conversationRequest(ctx, t, server, &bob, "POST", path+"/test", `{"modelId":"m"}`, 403),
		webapi.ErrorCodeForbidden,
	)
	const id = "3c1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a01"
	conversationCreate(ctx, t, server, &bob, id)
	conversationChoose(ctx, t, server, &bob, id, string(shared.ID), "m")
	socket, err := server.Conversation(ctx, t, &bob, id)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := socket.Open(ctx); err != nil {
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
	wireMust(t, socket.Close(ctx))
	if err := server.Close(ctx); err != nil {
		t.Fatal(err)
	}
	server, err = harness.StartInMode(ctx, t, webapi.InstanceModeIsolated)
	if err != nil {
		t.Fatal(err)
	}
	master, err = server.Login(ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	bob, err = server.Login(ctx, "bob@example.test", "bob-pass-1")
	wireMust(t, err)
	socket, err = server.Conversation(ctx, t, &bob, id)
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
	conversationEqual(t, *refused.Code, "provider_not_found")
	conversationRefusal(
		t,
		conversationRequest(
			ctx,
			t,
			server,
			&bob,
			"PATCH",
			"/api/conversations/"+id,
			`{"model":{"providerId":"`+string(shared.ID)+`","modelId":"m"}}`,
			404,
		),
		webapi.ErrorCodeProviderNotFound,
	)
	wireMust(t, socket.Close(ctx))
	conversationEqual(
		t,
		len(
			conversationDecode(
				t,
				conversationRequest(ctx, t, server, &master, "GET", "/api/providers", "", 200),
				webapi.DecodeProviders,
			).Providers,
		),
		1,
	)
	conversationEqual(
		t,
		len(
			conversationDecode(
				t,
				conversationRequest(ctx, t, server, &bob, "GET", "/api/providers", "", 200),
				webapi.DecodeProviders,
			).Providers,
		),
		0,
	)
}

// TestAVendorEntryTakesItsFamilyWireAndEndpointFromModelsDevAndReadsItsLiveModels
// checks vendor catalog configuration and live models.
func TestAVendorEntryTakesItsFamilyWireAndEndpointFromModelsDevAndReadsItsLiveModels(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	vendor := providertest.StartVendor(t)
	directory := &backendtest.AccountDirectory{}
	harness.Config.Families.Register(
		"openai",
		&backendtest.AccountFamily{
			T:         t,
			Directory: directory,
			WireTypes: []core.WireAPI{core.WireAPIResponses, core.WireAPIChatCompletions},
		},
	)
	var err error
	harness.Config.ModelsDevURL, err = url.Parse(vendor.URL("/api.json"))
	if err != nil {
		t.Fatal(err)
	}
	server, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	document := `{"deepseek":{"id":"deepseek","name":"DeepSeek","npm":"@ai-sdk/openai-compatible",` +
		`"api":"https://api.deepseek.com","doc":"https://api-docs.deepseek.com",` +
		`"models":{"deepseek-v4":{"name":"DeepSeek V4","reasoning":true,"tool_call":true,` +
		`"attachment":false,"limit":{"context":128000,"output":32000},"cost":{"input":0.3,` +
		`"output":1.2,"cache_read":0.03,"cache_write":0}},` +
		`"deepseek-v4-flash":{"name":"DeepSeek V4 Flash","limit":{"context":128000}}}},` +
		`"minimax":{"id":"minimax","name":"MiniMax","npm":"@ai-sdk/anthropic",` +
		`"api":"https://api.minimax.io/anthropic/v1","models":{"minimax-m3":{"name":"MiniMax M3",` +
		`"limit":{"context":200000,"output":64000}}}},"zai":{"id":"zai","name":"z.ai",` +
		`"npm":"@ai-sdk/google","models":{}},"amazon-bedrock":{"id":"amazon-bedrock",` +
		`"name":"Amazon Bedrock","npm":"@ai-sdk/amazon-bedrock",` +
		`"models":{"anthropic.claude-sonnet":{"name":"Claude Sonnet on Bedrock"}}},` +
		`"github-copilot":{"id":"github-copilot","name":"GitHub Copilot",` +
		`"npm":"@ai-sdk/openai-compatible","api":"https://api.githubcopilot.com",` +
		`"models":{"gpt-5.5":{"name":"GPT-5.5"}}}}`
	response := accountVendorResponse(document)
	response.Headers = http.Header{"Etag": []string{`"fixture"`}}
	vendor.Respond(response)
	offered := conversationDecode(
		t,
		conversationRequest(ctx, t, server, &master, "GET", "/api/providers/catalog", "", 200),
		webapi.DecodeVendorCatalog,
	)
	expected := conversationDecode(
		t,
		backendtest.Answer{
			Body: []byte(
				`{"vendors":[{"id":"deepseek","name":"DeepSeek","providerType":"openai",` +
					`"wireApi":"chat-completions","baseUrl":"https://api.deepseek.com",` +
					`"doc":"https://api-docs.deepseek.com"},{"id":"minimax","name":"MiniMax",` +
					`"providerType":"anthropic","baseUrl":"https://api.minimax.io/anthropic/v1","doc":null},` +
					`{"id":"zai","name":"z.ai","providerType":"google","baseUrl":null,"doc":null}],` +
					`"subscriptions":[]}`,
			),
		},
		webapi.DecodeVendorCatalog,
	)
	conversationEqual(t, offered.Vendors, expected.Vendors)
	conversationRefusal(
		t,
		conversationRequest(
			ctx,
			t,
			server,
			&master,
			"POST",
			"/api/providers",
			`{"source":"vendor","vendorId":"amazon-bedrock","label":"x","apiKey":"k"}`,
			400,
		),
		webapi.ErrorCodeUnknownVendor,
	)
	deepseek := accountProviderEntry(
		ctx,
		t,
		server,
		&master,
		`{"source":"vendor","vendorId":"deepseek","label":"DeepSeek","apiKey":"sk-1"}`,
	)
	conversationEqual(t, deepseek.ProviderType, "openai")
	conversationEqual(t, *deepseek.WireAPI, core.WireAPIChatCompletions)
	conversationEqual(t, *deepseek.VendorID, "deepseek")
	conversationEqual(t, string(*deepseek.BaseURL), "https://api.deepseek.com/")
	vendor.Respond(providertest.MockResponse{Status: 304})
	live := accountModels(ctx, t, server, &master, "")
	conversationEqual(t, vendor.Requests()[1].Header("if-none-match"), `"fixture"`)
	var ids []string
	for _, m := range live.Providers[0].Models {
		ids = append(ids, m.ID)
	}
	conversationEqual(t, ids, []string{"deepseek-v4", "deepseek-v4-flash"})
	v4 := live.Providers[0].Models[0]
	conversationEqual(t, *v4.ContextWindow, uint32(128000))
	conversationEqual(t, *v4.OutputLimit, uint32(32000))
	conversationEqual(t, *v4.SupportsReasoning, true)
	conversationEqual(t, string(v4.Selection.ProviderID), string(deepseek.ID))
	conversationEqual(t, v4.Selection.Model.ContextWindow, uint32(128000))
	conversationEqual(t, live.Providers[0].Stale, false)
	conversationEqual(t, directory.Reads(), 0)
	path := "/api/providers/" + string(deepseek.ID)
	conversationRequest(
		ctx,
		t,
		server,
		&master,
		"PATCH",
		path,
		`{"models":[`+configuredAccountModel(t, "deepseek-v4")+`]}`,
		200,
	)
	typed := accountModels(ctx, t, server, &master, "?refresh=true")
	conversationEqual(t, len(typed.Providers[0].Models), 1)
	conversationEqual(t, len(vendor.Requests()), 2)
	conversationRequest(ctx, t, server, &master, "PATCH", path, `{"models":null}`, 200)
	vendor.Respond(providertest.MockResponse{Status: 304})
	conversationEqual(t, len(accountModels(ctx, t, server, &master, "").Providers[0].Models), 2)
}

// TestAnAnthropicCompatibleVendorReceivesAnEffortAsATokenBudget
// checks effort settings in compatible vendor requests.
func TestAnAnthropicCompatibleVendorReceivesAnEffortAsATokenBudget(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	vendor := providertest.StartVendor(t)
	vendor.RespondAt(
		"/api.json",
		accountVendorResponse(
			`{"moonshot":{"id":"moonshot","name":"Moonshot","npm":"@ai-sdk/anthropic",`+
				`"api":"https://api.moonshot.example/anthropic","models":{}}}`,
		),
	)
	var err error
	harness.Config.ModelsDevURL, err = url.Parse(vendor.URL("/api.json"))
	if err != nil {
		t.Fatal(err)
	}
	server, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	model := `{"id":"m","displayName":"M","contextWindow":100000,"outputLimit":64000,` +
		`"thinkingEfforts":["low","high"],"acceptedExtensions":null,"fastTier":null}`
	compatible := accountProviderEntry(
		ctx,
		t,
		server,
		&master,
		`{"source":"vendor","vendorId":"moonshot","label":"Moonshot","apiKey":"key","baseUrl":"`+vendor.URL(
			"/v1",
		)+`","models":[`+model+`]}`,
	)
	anthropic := accountProviderEntry(
		ctx,
		t,
		server,
		&master,
		`{"source":"custom","providerType":"anthropic","label":"Anthropic","apiKey":"key","baseUrl":"`+vendor.URL(
			"/v1",
		)+`","models":[`+model+`]}`,
	)
	const id = "3c1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a01"
	conversationCreate(ctx, t, server, &master, id)
	var sent []map[string]any
	for i, entry := range []webapi.ProviderDTO{compatible, anthropic} {
		conversationRequest(
			ctx,
			t,
			server,
			&master,
			"PATCH",
			"/api/conversations/"+id,
			`{"model":{"providerId":"`+string(entry.ID)+`","modelId":"m"},"thinkingEffort":"high"}`,
			200,
		)
		socket, err := server.Conversation(ctx, t, &master, id)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := socket.Open(ctx); err != nil {
			t.Fatal(err)
		}
		vendor.Respond(conversationAnswer(t, []string{"Done."}, 1, 1))
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
		wireMust(t, socket.Close(ctx))
	}
	conversationEqual(t, len(sent), 2)
	conversationEqual(t, sent[0]["thinking"], any(map[string]any{"type": "enabled", "budget_tokens": float64(32768)}))
	if _, ok := sent[0]["output_config"]; ok {
		t.Fatal(sent[0])
	}
	thinking, ok := sent[1]["thinking"].(map[string]any)
	if !ok {
		t.Fatal("Anthropic request has no thinking object")
	}
	conversationEqual(t, thinking["type"], any("adaptive"))
	conversationEqual(t, sent[1]["output_config"], any(map[string]any{"effort": "high"}))
}

// anthropicEvent emits one Messages API event with its typed name.
func anthropicEvent(name, body string) string {
	return "event: " + name + "\ndata: " + body + "\n\n"
}

// conversationMessageStart begins the scripted vendor's response.
func conversationMessageStart(usage string) string {
	return anthropicEvent(
		"message_start",
		`{"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant",`+
			`"model":"claude-opus-4-8","content":[],"usage":`+usage+`}}`,
	)
}

// conversationTextBlock streams each text delta independently.
func conversationTextBlock(t *testing.T, index int, texts ...string) string {
	result := anthropicEvent(
		"content_block_start",
		fmt.Sprintf(`{"type":"content_block_start","index":%d,"content_block":{"type":"text","text":""}}`, index),
	)
	for _, text := range texts {
		result += anthropicEvent(
			"content_block_delta",
			fmt.Sprintf(
				`{"type":"content_block_delta","index":%d,"delta":{"type":"text_delta","text":%s}}`,
				index,
				conversationJSON(t, text),
			),
		)
	}
	return result + anthropicEvent("content_block_stop", fmt.Sprintf(`{"type":"content_block_stop","index":%d}`, index))
}
