package backend_test

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/providers/openaiapi"
	"github.com/wspl/demi/internal/webapi"
)

// conversationPage opens a page and consumes its initial snapshot.
func conversationPage(ctx context.Context, t *testing.T, b *backendtest.TestBackend, s *backendtest.Session) (*backendtest.SyncChannel, webapi.ProductState) {
	t.Helper()
	page, err := b.Sync(ctx, t, s)
	wireMust(t, err)
	state, err := page.Snapshot(ctx)
	wireMust(t, err)
	return page, state
}

// conversationEvent reads a validated page event.
func conversationEvent(ctx context.Context, t *testing.T, page *backendtest.SyncChannel) webapi.SyncEvent {
	t.Helper()
	e, err := page.Next(ctx)
	wireMust(t, err)
	return e
}

// conversationChanged extracts the changed conversation carried by a page event.
func conversationChanged(t *testing.T, event webapi.SyncEvent) webapi.ConversationSummary {
	t.Helper()
	c, ok := event.(*webapi.SyncEventConversation)
	if !ok {
		t.Fatalf("expected conversation, got %T", event)
	}
	conversationEqual(t, string(c.Conversation.ID), conversationFirst)
	return c.Conversation
}

// conversationTheme changes the preference through the page's HTTP API.
func conversationTheme(ctx context.Context, t *testing.T, b *backendtest.TestBackend, s *backendtest.Session, theme string) {
	t.Helper()
	conversationRequest(ctx, t, b, s, "PATCH", "/api/settings/preferences", `{"appearance":{"theme":"`+theme+`"}}`, 200)
}

// conversationThemed checks the theme carried by the next preference event.
func conversationThemed(t *testing.T, event webapi.SyncEvent, theme webapi.Theme) {
	t.Helper()
	p, ok := event.(*webapi.SyncEventPreferences)
	if !ok {
		t.Fatalf("expected preferences, got %T", event)
	}
	conversationEqual(t, p.Preferences.Appearance.Theme, &theme)
}

// A local page sees all initial product state without waking a runner.
func TestSnapshotIsUsersProductState(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	_, state := conversationPage(ctx, t, b, &s)
	want := webapi.ProductState{
		User: s.User, Mode: webapi.InstanceModeShared, Preferences: webapi.Preferences{},
		Providers: []webapi.ProviderState{}, Workspaces: []webapi.WorkspaceDTO{}, Devices: []webapi.DeviceDTO{},
		PublicURL: b.URL + "/", Conversations: []webapi.ConversationSummary{},
		Cloud:   webapi.CloudStatus{State: webapi.CloudStateUnallocated, Limits: webapi.CloudVolumes{SystemBytes: 16 << 30, HomeBytes: 32 << 30}},
		Plugins: state.Plugins, PluginStates: map[string]json.RawMessage{"expose": json.RawMessage(`{"available":false,"exposes":[]}`), "skills": json.RawMessage(`{"sources":[]}`)},
	}
	conversationEqual(t, state, want)
	var names []string
	for _, p := range state.Plugins {
		names = append(names, string(p.ID))
		conversationEqual(t, p.Enabled, true)
	}
	conversationEqual(t, names, []string{"file", "todo", "browser", "expose", "skills", "changes", "file-browser"})
	conversationRequest(ctx, t, b, nil, "GET", "/install.sh", "", 503)
}

func TestSessionChangesReachOtherPageOnce(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	b, laptop, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	phone, err := b.Login(ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	conversationCreate(ctx, t, b, &laptop, conversationFirst)
	page, state := conversationPage(ctx, t, b, &phone)
	conversationEqual(t, state.Conversations[0].Title, "New conversation")
	conversationEqual(t, state.Conversations[0].DraftRevision, uint64(0))
	conversationRequest(ctx, t, b, &laptop, "PATCH", "/api/conversations/"+conversationFirst, `{"title":"Fix the login"}`, 200)
	conversationEqual(t, conversationChanged(t, conversationEvent(ctx, t, page)).Title, "Fix the login")
	conversationRequest(ctx, t, b, &laptop, "PUT", "/api/conversations/"+conversationFirst+"/draft", `{"base":0,"text":"The login fails","files":[]}`, 200)
	conversationEqual(t, conversationChanged(t, conversationEvent(ctx, t, page)).DraftRevision, uint64(1))
	conversationRequest(ctx, t, b, &laptop, "PATCH", "/api/auth/me", `{"nickname":"Ana"}`, 200)
	user, ok := conversationEvent(ctx, t, page).(*webapi.SyncEventUser)
	if !ok {
		t.Fatal("nickname did not send user event")
	}
	conversationEqual(t, user.User.Nickname, "Ana")
	conversationTheme(ctx, t, b, &laptop, "dark")
	conversationThemed(t, conversationEvent(ctx, t, page), webapi.ThemeDark)
}

func TestReconnectingPageCatchesChangesDuringSnapshot(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	b, laptop, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	phone, err := b.Login(ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	conversationCreate(ctx, t, b, &laptop, conversationFirst)
	away, _ := conversationPage(ctx, t, b, &phone)
	wireMust(t, away.Close(ctx))
	conversationRequest(ctx, t, b, &laptop, "PATCH", "/api/conversations/"+conversationFirst, `{"title":"Renamed while away"}`, 200)
	held := backendtest.HoldSync(t, b.Backend, backendtest.SyncSnapshot)
	page, err := b.Sync(ctx, t, &phone)
	wireMust(t, err)
	wireMust(t, held.UntilArrived(ctx, 1))
	conversationRequest(ctx, t, b, &laptop, "PATCH", "/api/conversations/"+conversationFirst, `{"pinned":true}`, 200)
	held.Release()
	state, err := page.Snapshot(ctx)
	wireMust(t, err)
	conversationEqual(t, state.Conversations[0].Title, "Renamed while away")
	conversationEqual(t, state.Conversations[0].Pinned, false)
	caught, err := page.Until(ctx, func(e webapi.SyncEvent) bool {
		_, ok := e.(*webapi.SyncEventConversationOrder)
		return ok
	})
	wireMust(t, err)
	conversationEqual(t, len(caught), 2)
	conversationEqual(t, conversationChanged(t, caught[0]).Pinned, true)
}

func TestLaggingPageReceivesEachCurrentPartOnce(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	b, laptop, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	phone, err := b.Login(ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	conversationCreate(ctx, t, b, &laptop, conversationFirst)
	page, _ := conversationPage(ctx, t, b, &phone)
	held := backendtest.HoldSync(t, b.Backend, backendtest.SyncChanges)
	for i := 1; i <= 50; i++ {
		conversationRequest(ctx, t, b, &laptop, "PATCH", "/api/conversations/"+conversationFirst, fmt.Sprintf(`{"title":"Title %d"}`, i), 200)
	}
	conversationTheme(ctx, t, b, &laptop, "dark")
	wireMust(t, held.UntilArrived(ctx, 1))
	held.Release()
	conversationEqual(t, conversationChanged(t, conversationEvent(ctx, t, page)).Title, "Title 50")
	conversationThemed(t, conversationEvent(ctx, t, page), webapi.ThemeDark)
	conversationRequest(ctx, t, b, &laptop, "PATCH", "/api/conversations/"+conversationFirst, `{"title":"Caught up"}`, 200)
	conversationEqual(t, conversationChanged(t, conversationEvent(ctx, t, page)).Title, "Caught up")
}

// conversationUpgradeRefusal makes an HTTP upgrade whose refusal is observable.
func conversationUpgradeRefusal(ctx context.Context, t *testing.T, b *backendtest.TestBackend, s *backendtest.Session, path, origin string, status int, code webapi.ErrorCode) {
	t.Helper()
	headers := http.Header{"Upgrade": {"websocket"}, "Connection": {"Upgrade"}, "Sec-Websocket-Version": {"13"}, "Sec-Websocket-Key": {"MDEyMzQ1Njc4OWFiY2RlZg=="}, "Origin": {origin}}
	response, err := b.Response(ctx, "GET", path, s, headers, nil)
	wireMust(t, err)
	a, err := backendtest.ReadAnswer(ctx, response)
	wireMust(t, err)
	conversationEqual(t, a.Status, status)
	conversationRefusal(t, a, code)
}

func TestPageChannelOriginAuthenticationAndLifetime(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	b, laptop, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	conversationUpgradeRefusal(ctx, t, b, &laptop, "/api/sync", "https://a1b2c3.expose.localhost", 403, webapi.ErrorCodeForbiddenOrigin)
	conversationUpgradeRefusal(ctx, t, b, nil, "/api/sync", b.URL, 401, webapi.ErrorCodeUnauthenticated)
	plain, err := b.ReadWith(ctx, "/api/sync", &laptop, http.Header{"Origin": {b.URL}})
	wireMust(t, err)
	conversationEqual(t, plain.Status, 426)
	conversationRefusal(t, plain, webapi.ErrorCodeUpgradeRequired)
	phone, err := b.Login(ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	tablet, err := b.Login(ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	laptopPage, _ := conversationPage(ctx, t, b, &laptop)
	phonePage, _ := conversationPage(ctx, t, b, &phone)
	conversationRequest(ctx, t, b, &phone, "POST", "/api/auth/logout", `{}`, 204)
	closed := func(page *backendtest.SyncChannel, code uint16, reason string) {
		got, message, err := page.Closed(ctx)
		wireMust(t, err)
		conversationEqual(t, got, code)
		conversationEqual(t, message, reason)
	}
	closed(phonePage, 4002, "session_ended")
	wireMust(t, h.Clock.Advance(20*24*time.Hour))
	conversationTheme(ctx, t, b, &laptop, "dark")
	conversationThemed(t, conversationEvent(ctx, t, laptopPage), webapi.ThemeDark)
	tabletPage, _ := conversationPage(ctx, t, b, &tablet)
	wireMust(t, h.Clock.Advance(11*24*time.Hour))
	conversationTheme(ctx, t, b, &laptop, "light")
	conversationThemed(t, conversationEvent(ctx, t, laptopPage), webapi.ThemeLight)
	closed(tabletPage, 4002, "session_ended")
	wireMust(t, laptopPage.SendText(ctx, "hello"))
	closed(laptopPage, 1008, "unexpected_message")
	last, _ := conversationPage(ctx, t, b, &laptop)
	done := make(chan error, 1)
	go func() { done <- b.Close(ctx) }()
	defer func() { wireMust(t, <-done) }()
	closed(last, 1001, "backend_closing")
}

// Two real sockets wait for their first heartbeat (200 ms); no polling or sleep.
func TestPageSocketsHeartbeatAfterQuietInterval(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	h.Config.Pages.Heartbeat = 200 * time.Millisecond
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	conversationCreate(ctx, t, b, &s, conversationFirst)
	connected := time.Now()
	page, err := b.Sync(ctx, t, &s)
	wireMust(t, err)
	socket, err := b.Conversation(ctx, t, &s, conversationFirst)
	wireMust(t, err)
	_, err = page.Snapshot(ctx)
	wireMust(t, err)
	if _, ok := conversationEvent(ctx, t, page).(*webapi.SyncEventHeartbeat); !ok {
		t.Fatal("expected page heartbeat")
	}
	frame, err := socket.Next(ctx)
	wireMust(t, err)
	if _, ok := frame.(*framewire.HeartbeatFrame); !ok {
		t.Fatalf("expected heartbeat, got %T", frame)
	}
	if time.Since(connected) < h.Config.Pages.Heartbeat {
		t.Fatal("heartbeat before quiet interval")
	}
}

func TestTurnAndReadStateReachEveryPage(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	vendor := providertest.StartVendor(t)
	b, laptop, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	phone, err := b.Login(ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	provider := conversationAnthropic(ctx, t, b, &laptop, vendor)
	conversationCreate(ctx, t, b, &laptop, conversationFirst)
	conversationChoose(ctx, t, b, &laptop, conversationFirst, provider, "claude-opus-4-8")
	page, _ := conversationPage(ctx, t, b, &phone)
	pending := conversationAnswer(t, []string{"Hello"}, 12, 0)
	text := string(pending.Chunks[0])
	text = text[:strings.Index(text, "event: content_block_stop")]
	pending.Chunks = [][]byte{[]byte(text)}
	pending.Ending = providertest.Open
	vendor.Respond(pending)
	socket := conversationOpen(ctx, t, b, &laptop, conversationFirst)
	wireMust(t, socket.Send(ctx, backendtest.ConversationText("m1", "Say hello")))
	until := func(want func(webapi.ConversationSummary) bool) webapi.ConversationSummary {
		events, err := page.Until(ctx, func(e webapi.SyncEvent) bool {
			c, ok := e.(*webapi.SyncEventConversation)
			return ok && c.Conversation.ID == conversationFirst && want(c.Conversation)
		})
		wireMust(t, err)
		return conversationChanged(t, events[len(events)-1])
	}
	until(func(c webapi.ConversationSummary) bool { return c.Status == webapi.ConversationStatusRunning })
	wireMust(t, socket.Send(ctx, &framewire.AbortFrame{}))
	stopped := until(func(c webapi.ConversationSummary) bool { return c.Status == webapi.ConversationStatusStopped })
	if !stopped.Unread || stopped.Revision == 0 {
		t.Fatalf("stopped summary: %#v", stopped)
	}
	conversationRequest(ctx, t, b, &laptop, "POST", "/api/conversations/"+conversationFirst+"/read", fmt.Sprintf(`{"revision":%d}`, stopped.Revision), 204)
	until(func(c webapi.ConversationSummary) bool { return !c.Unread })
}

// pageLoginKit supplies the device login result without contacting a vendor.
type pageLoginKit struct{}

func (pageLoginKit) Capability() provider.AccountsCapability {
	return provider.AccountsCapability{Login: true, Add: true}
}
func (pageLoginKit) Login(ctx context.Context, pending func(core.LoginPending)) (provider.NewAccount, error) {
	code := "ABCD-1234"
	pending(core.LoginPending{VerificationURL: "https://verify.example/device", UserCode: &code})
	if err := ctx.Err(); err != nil {
		return provider.NewAccount{}, err
	}
	identity := "device"
	return provider.NewAccount{Secret: `{"token":"login-secret"}`, Label: provider.AccountLabel{Label: "device@example.test", IdentityKey: &identity}}, nil
}
func (pageLoginKit) Add(provider.AddAccount) (provider.NewAccount, error) {
	return provider.NewAccount{}, provider.ErrAccountsUnsupported
}

type pageSubscriptionFamily struct{}

func (pageSubscriptionFamily) Credential() webapi.CredentialKind {
	return webapi.CredentialKindSubscription
}
func (pageSubscriptionFamily) Wires() []core.WireAPI { return nil }
func (pageSubscriptionFamily) Provider(args providers.FamilyArgs) (provider.Provider, error) {
	subscription, ok := args.Credential.(*providers.SubscriptionArgs)
	if !ok {
		return nil, &providers.FamilyError{Kind: providers.FamilyWrongCredential}
	}
	p := &pageSubscription{Provider: openaiapi.New(openaiapi.Config{APIKey: "fixture"}, args.Clock), accounts: provider.NewAccounts(subscription.Pool, pageLoginKit{}, args.Clock)}
	if subscription.Account != nil {
		p.account = &subscription.Account.CredentialID
		p.quota = provider.NewQuota(pageQuota{}, subscription.Account.Quota, args.Clock)
	}
	return p, nil
}

type pageSubscription struct {
	*openaiapi.Provider
	accounts provider.SubscriptionAccounts
	account  *string
	quota    *provider.Quota
}

func (p *pageSubscription) Accounts() provider.SubscriptionAccounts { return p.accounts }
func (p *pageSubscription) AuthStatus(context.Context) core.AuthState {
	return &core.Authenticated{AccountLabel: p.account}
}
func (p *pageSubscription) Quota() *provider.Quota { return p.quota }

type pageQuota struct{}

func (pageQuota) ProbeCost() *provider.ProbeCost { cost := provider.ProbeFree; return &cost }
func (pageQuota) Probe(context.Context) (provider.ProbeReading, error) {
	percent := float64(40)
	label := "device@example.test"
	return provider.ProbeReading{AccountLabel: &label, Windows: []core.QuotaWindow{{ID: "weekly", Label: "Weekly", UsedPercent: &percent}}}, nil
}
func (pageQuota) Observe(provider.Observation) []core.QuotaWindow { return nil }

func TestProviderChangesReachAllUsersWithAccountsOnlyForConfigurer(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	h.Config.Families.Register("device", pageSubscriptionFamily{})
	h.Config.Families.Register("claude-code", pageSubscriptionFamily{})
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	wireMust(t, h.AddUser(ctx, "reader@example.test", "reader-pass-1", webapi.RoleUser))
	reader, err := b.Login(ctx, "reader@example.test", "reader-pass-1")
	wireMust(t, err)
	masters, state := conversationPage(ctx, t, b, &s)
	conversationEqual(t, len(state.Providers), 0)
	readers, _ := conversationPage(ctx, t, b, &reader)
	started := conversationDecode(t, conversationRequest(ctx, t, b, &s, "POST", "/api/providers/subscription-login", `{"providerType":"device"}`, 202), webapi.DecodeLoginStarted)
	// Publication of the new provider is the event that the login has committed.
	_, err = masters.Until(ctx, func(e webapi.SyncEvent) bool {
		p, ok := e.(*webapi.SyncEventProviders)
		return ok && len(p.Providers) == 1
	})
	wireMust(t, err)
	login := conversationDecode(t, conversationRequest(ctx, t, b, &s, "GET", "/api/providers/subscription-login/"+string(started.Login.ID), "", 200), webapi.DecodeLoginAnswer)
	completed, ok := login.Login.(*webapi.LoginStateCompleted)
	if !ok {
		t.Fatalf("login did not complete: %v", login.Login)
	}
	conversationRequest(ctx, t, b, &s, "POST", "/api/providers", `{"source":"custom","providerType":"anthropic","label":"Work","apiKey":"sk-state"}`, 201)
	until := func(page *backendtest.SyncChannel) []webapi.ProviderState {
		events, err := page.Until(ctx, func(e webapi.SyncEvent) bool {
			p, ok := e.(*webapi.SyncEventProviders)
			return ok && len(p.Providers) == 2
		})
		wireMust(t, err)
		return events[len(events)-1].(*webapi.SyncEventProviders).Providers
	}
	details := func(state webapi.ProviderState) webapi.ProviderDetails {
		d, ok := state.Details.(*webapi.ProviderReadingRead)
		if !ok {
			t.Fatalf("provider cannot be read: %v", state.Details)
		}
		return d.ProviderDetails
	}
	seen := until(masters)
	conversationEqual(t, []string{seen[0].Label, seen[1].Label}, []string{"device subscription", "Work"})
	if strings.Contains(conversationJSON(t, seen), "sk-state") {
		t.Fatal("page disclosed API key")
	}
	subscription := details(seen[0])
	conversationEqual(t, len(subscription.Accounts), 1)
	conversationEqual(t, string(*subscription.Active), subscription.Accounts[0].ID)
	if _, ok := details(seen[1]).Auth.(*core.Authenticated); !ok {
		t.Fatal("key entry not authenticated")
	}
	conversationEqual(t, seen[0].ID, completed.ProviderID)
	seen = until(readers)
	subscription = details(seen[0])
	conversationEqual(t, len(subscription.Accounts), 0)
	conversationEqual(t, subscription.Active, (*webapi.CredentialID)(nil))
	conversationEqual(t, subscription.Quota, (*core.QuotaSnapshot)(nil))
	conversationEqual[core.AuthState](t, subscription.Auth, &core.Authenticated{})
}
