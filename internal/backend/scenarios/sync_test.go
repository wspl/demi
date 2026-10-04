package scenarios_test

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/backend/providerhost"
	"github.com/wspl/demi/internal/conversationproto"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/openaiapi"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// conversationPage opens a page and consumes its initial snapshot.
func conversationPage(
	ctx context.Context,
	t *testing.T,
	backend *backendtest.TestBackend,
	session *backendtest.Session,
) (*backendtest.SyncChannel, webapiproto.ProductState) {
	t.Helper()
	page, err := backend.Sync(ctx, t, session)
	wireMust(t, err)
	state, err := page.Snapshot(ctx)
	wireMust(t, err)
	return page, state
}

// conversationEvent reads a validated page event.
func conversationEvent(ctx context.Context, t *testing.T, page *backendtest.SyncChannel) webapiproto.SyncEvent {
	t.Helper()
	e, err := page.Next(ctx)
	wireMust(t, err)
	return e
}

// conversationChanged extracts the changed conversation carried by a page event.
func conversationChanged(t *testing.T, event webapiproto.SyncEvent) webapiproto.ConversationSummary {
	t.Helper()
	c, ok := event.(*webapiproto.SyncEventConversation)
	if !ok {
		t.Fatalf("expected conversation, got %T", event)
	}
	conversationEqual(t, string(c.Conversation.ID), conversationFirst)
	return c.Conversation
}

// conversationTheme changes the preference through the page's HTTP API.
func conversationTheme(
	ctx context.Context,
	t *testing.T,
	backend *backendtest.TestBackend,
	session *backendtest.Session,
	theme string,
) {
	t.Helper()
	conversationRequest(
		ctx,
		t,
		backend,
		session,
		"PATCH",
		"/api/settings/preferences",
		`{"appearance":{"theme":"`+theme+`"}}`,
		200,
	)
}

// conversationThemed checks the theme carried by the next preference event.
func conversationThemed(t *testing.T, event webapiproto.SyncEvent, theme webapiproto.Theme) {
	t.Helper()
	p, ok := event.(*webapiproto.SyncEventPreferences)
	if !ok {
		t.Fatalf("expected preferences, got %T", event)
	}
	conversationEqual(t, p.Preferences.Appearance.Theme, &theme)
}

// TestSnapshotIsUsersProductState checks that the initial snapshot contains the user's product state.
// A local page sees all initial product state without waking a runner.
func TestSnapshotIsUsersProductState(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	_, state := conversationPage(ctx, t, backend, &session)
	want := webapiproto.ProductState{
		User:          session.User,
		Mode:          webapiproto.InstanceModeShared,
		Preferences:   webapiproto.Preferences{},
		Providers:     []webapiproto.ProviderState{},
		Workspaces:    []webapiproto.WorkspaceDTO{},
		Devices:       []webapiproto.DeviceDTO{},
		PublicURL:     backend.URL + "/",
		Conversations: []webapiproto.ConversationSummary{},
		Cloud: webapiproto.CloudStatus{
			State:  webapiproto.CloudStateUnallocated,
			Limits: webapiproto.CloudVolumes{SystemBytes: 16 << 30, HomeBytes: 32 << 30},
		},
		Plugins: state.Plugins,
		PluginStates: map[string]json.RawMessage{
			"expose": json.RawMessage(`{"available":false,"exposes":[]}`),
			"skills": json.RawMessage(`{"sources":[]}`),
		},
	}
	conversationEqual(t, state, want)
	var names []string
	for _, p := range state.Plugins {
		names = append(names, string(p.ID))
		conversationEqual(t, p.Enabled, true)
	}
	conversationEqual(t, names, []string{"file", "todo", "browser", "expose", "skills", "changes", "file-browser"})
	conversationRequest(ctx, t, backend, nil, "GET", "/install.sh", "", 503)
}

// TestSessionChangesReachOtherPageOnce checks that session changes reach another page once.
func TestSessionChangesReachOtherPageOnce(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	backend, laptop, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	phone, err := backend.Login(ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	conversationCreate(ctx, t, backend, &laptop, conversationFirst)
	page, state := conversationPage(ctx, t, backend, &phone)
	conversationEqual(t, state.Conversations[0].Title, "New conversation")
	conversationEqual(t, state.Conversations[0].DraftRevision, uint64(0))
	conversationRequest(
		ctx,
		t,
		backend,
		&laptop,
		"PATCH",
		"/api/conversations/"+conversationFirst,
		`{"title":"Fix the login"}`,
		200,
	)
	conversationEqual(t, conversationChanged(t, conversationEvent(ctx, t, page)).Title, "Fix the login")
	conversationRequest(
		ctx,
		t,
		backend,
		&laptop,
		"PUT",
		"/api/conversations/"+conversationFirst+"/draft",
		`{"base":0,"text":"The login fails","files":[]}`,
		200,
	)
	conversationEqual(t, conversationChanged(t, conversationEvent(ctx, t, page)).DraftRevision, uint64(1))
	conversationRequest(ctx, t, backend, &laptop, "PATCH", "/api/auth/me", `{"nickname":"Ana"}`, 200)
	user, ok := conversationEvent(ctx, t, page).(*webapiproto.SyncEventUser)
	if !ok {
		t.Fatal("nickname did not send user event")
	}
	conversationEqual(t, user.User.Nickname, "Ana")
	conversationTheme(ctx, t, backend, &laptop, "dark")
	conversationThemed(t, conversationEvent(ctx, t, page), webapiproto.ThemeDark)
}

// TestReconnectingPageCatchesChangesDuringSnapshot checks that reconnecting pages receive changes made
// during their snapshot.
func TestReconnectingPageCatchesChangesDuringSnapshot(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	backend, laptop, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	phone, err := backend.Login(ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	conversationCreate(ctx, t, backend, &laptop, conversationFirst)
	away, _ := conversationPage(ctx, t, backend, &phone)
	wireMust(t, away.Close(ctx))
	conversationRequest(
		ctx,
		t,
		backend,
		&laptop,
		"PATCH",
		"/api/conversations/"+conversationFirst,
		`{"title":"Renamed while away"}`,
		200,
	)
	held := backendtest.HoldSync(t, backend.Backend, backendtest.SyncSnapshot)
	page, err := backend.Sync(ctx, t, &phone)
	wireMust(t, err)
	wireMust(t, held.UntilArrived(ctx, 1))
	conversationRequest(
		ctx,
		t,
		backend,
		&laptop,
		"PATCH",
		"/api/conversations/"+conversationFirst,
		`{"pinned":true}`,
		200,
	)
	held.Release()
	state, err := page.Snapshot(ctx)
	wireMust(t, err)
	conversationEqual(t, state.Conversations[0].Title, "Renamed while away")
	conversationEqual(t, state.Conversations[0].Pinned, false)
	caught, err := page.Until(ctx, func(e webapiproto.SyncEvent) bool {
		_, ok := e.(*webapiproto.SyncEventConversationOrder)
		return ok
	})
	wireMust(t, err)
	conversationEqual(t, len(caught), 2)
	conversationEqual(t, conversationChanged(t, caught[0]).Pinned, true)
}

// TestLaggingPageReceivesEachCurrentPartOnce checks that lagging pages receive each current state part
// once.
func TestLaggingPageReceivesEachCurrentPartOnce(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	backend, laptop, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	phone, err := backend.Login(ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	conversationCreate(ctx, t, backend, &laptop, conversationFirst)
	page, _ := conversationPage(ctx, t, backend, &phone)
	held := backendtest.HoldSync(t, backend.Backend, backendtest.SyncChanges)
	for i := 1; i <= 50; i++ {
		conversationRequest(
			ctx,
			t,
			backend,
			&laptop,
			"PATCH",
			"/api/conversations/"+conversationFirst,
			fmt.Sprintf(`{"title":"Title %d"}`, i),
			200,
		)
	}
	conversationTheme(ctx, t, backend, &laptop, "dark")
	wireMust(t, held.UntilArrived(ctx, 1))
	held.Release()
	conversationEqual(t, conversationChanged(t, conversationEvent(ctx, t, page)).Title, "Title 50")
	conversationThemed(t, conversationEvent(ctx, t, page), webapiproto.ThemeDark)
	conversationRequest(
		ctx,
		t,
		backend,
		&laptop,
		"PATCH",
		"/api/conversations/"+conversationFirst,
		`{"title":"Caught up"}`,
		200,
	)
	conversationEqual(t, conversationChanged(t, conversationEvent(ctx, t, page)).Title, "Caught up")
}

// conversationUpgradeRefusal makes an HTTP upgrade whose refusal is observable.
func conversationUpgradeRefusal(
	ctx context.Context,
	t *testing.T,
	backend *backendtest.TestBackend,
	session *backendtest.Session,
	path, origin string,
	status int,
	code webapiproto.ErrorCode,
) {
	t.Helper()
	headers := http.Header{
		"Upgrade":               {"websocket"},
		"Connection":            {"Upgrade"},
		"Sec-Websocket-Version": {"13"},
		"Sec-Websocket-Key":     {"MDEyMzQ1Njc4OWFiY2RlZg=="},
		"Origin":                {origin},
	}
	response, err := backend.Response(ctx, "GET", path, session, headers, nil)
	wireMust(t, err)
	a, err := backendtest.ReadAnswer(ctx, response)
	wireMust(t, err)
	conversationEqual(t, a.Status, status)
	conversationRefusal(t, a, code)
}

// TestPageChannelOriginAuthenticationAndLifetime checks that page channels enforce origin,
// authentication and session lifetime.
func TestPageChannelOriginAuthenticationAndLifetime(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	backend, laptop, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	conversationUpgradeRefusal(
		ctx,
		t,
		backend,
		&laptop,
		"/api/sync",
		"https://a1b2c3.expose.localhost",
		403,
		webapiproto.ErrorCodeForbiddenOrigin,
	)
	conversationUpgradeRefusal(
		ctx,
		t,
		backend,
		nil,
		"/api/sync",
		backend.URL,
		401,
		webapiproto.ErrorCodeUnauthenticated,
	)
	plain, err := backend.ReadWith(ctx, "/api/sync", &laptop, http.Header{"Origin": {backend.URL}})
	wireMust(t, err)
	conversationEqual(t, plain.Status, 426)
	conversationRefusal(t, plain, webapiproto.ErrorCodeUpgradeRequired)
	phone, err := backend.Login(ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	tablet, err := backend.Login(ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	laptopPage, _ := conversationPage(ctx, t, backend, &laptop)
	phonePage, _ := conversationPage(ctx, t, backend, &phone)
	conversationRequest(ctx, t, backend, &phone, "POST", "/api/auth/logout", `{}`, 204)
	closed := func(page *backendtest.SyncChannel, code uint16, reason string) {
		got, message, err := page.Closed(ctx)
		wireMust(t, err)
		conversationEqual(t, got, code)
		conversationEqual(t, message, reason)
	}
	closed(phonePage, 4002, "session_ended")
	wireMust(t, harness.Clock.Advance(20*24*time.Hour))
	conversationTheme(ctx, t, backend, &laptop, "dark")
	conversationThemed(t, conversationEvent(ctx, t, laptopPage), webapiproto.ThemeDark)
	tabletPage, _ := conversationPage(ctx, t, backend, &tablet)
	wireMust(t, harness.Clock.Advance(11*24*time.Hour))
	conversationTheme(ctx, t, backend, &laptop, "light")
	conversationThemed(t, conversationEvent(ctx, t, laptopPage), webapiproto.ThemeLight)
	closed(tabletPage, 4002, "session_ended")
	wireMust(t, laptopPage.SendText(ctx, "hello"))
	closed(laptopPage, 1008, "unexpected_message")
	last, _ := conversationPage(ctx, t, backend, &laptop)
	done := make(chan error, 1)
	go func() { done <- backend.Close(ctx) }()
	defer func() { wireMust(t, <-done) }()
	closed(last, 1001, "backend_closing")
}

// TestPageSocketsHeartbeatAfterQuietInterval checks that quiet page sockets receive heartbeats.
// Two real sockets wait for their first heartbeat (200 ms); no polling or sleep.
func TestPageSocketsHeartbeatAfterQuietInterval(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	harness.Config.Pages.Heartbeat = 200 * time.Millisecond
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	connected := time.Now()
	page, err := backend.Sync(ctx, t, &session)
	wireMust(t, err)
	socket, err := backend.Conversation(ctx, t, &session, conversationFirst)
	wireMust(t, err)
	_, err = page.Snapshot(ctx)
	wireMust(t, err)
	if _, ok := conversationEvent(ctx, t, page).(*webapiproto.SyncEventHeartbeat); !ok {
		t.Fatal("expected page heartbeat")
	}
	frame, err := socket.Next(ctx)
	wireMust(t, err)
	if _, ok := frame.(*conversationproto.HeartbeatFrame); !ok {
		t.Fatalf("expected heartbeat, got %T", frame)
	}
	if time.Since(connected) < harness.Config.Pages.Heartbeat {
		t.Fatal("heartbeat before quiet interval")
	}
}

// TestTurnAndReadStateReachEveryPage checks that turn and read state changes reach every page.
func TestTurnAndReadStateReachEveryPage(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	vendor := providertest.StartVendor(t)
	backend, laptop, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	phone, err := backend.Login(ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	provider := conversationAnthropic(ctx, t, backend, &laptop, vendor)
	conversationCreate(ctx, t, backend, &laptop, conversationFirst)
	conversationChoose(ctx, t, backend, &laptop, conversationFirst, provider, "claude-opus-4-8")
	page, _ := conversationPage(ctx, t, backend, &phone)
	pending := conversationAnswer(t, []string{"Hello"}, 12, 0)
	text := string(pending.Chunks[0])
	text = text[:strings.Index(text, "event: content_block_stop")]
	pending.Chunks = [][]byte{[]byte(text)}
	pending.Ending = providertest.Open
	vendor.Respond(pending)
	socket := conversationOpen(ctx, t, backend, &laptop, conversationFirst)
	wireMust(t, socket.Send(ctx, backendtest.ConversationText("m1", "Say hello")))
	until := func(want func(webapiproto.ConversationSummary) bool) webapiproto.ConversationSummary {
		events, err := page.Until(ctx, func(e webapiproto.SyncEvent) bool {
			c, ok := e.(*webapiproto.SyncEventConversation)
			return ok && c.Conversation.ID == conversationFirst && want(c.Conversation)
		})
		wireMust(t, err)
		return conversationChanged(t, events[len(events)-1])
	}
	until(func(c webapiproto.ConversationSummary) bool { return c.Status == webapiproto.ConversationStatusRunning })
	wireMust(t, socket.Send(ctx, &conversationproto.AbortFrame{}))
	stopped := until(
		func(c webapiproto.ConversationSummary) bool { return c.Status == webapiproto.ConversationStatusStopped },
	)
	if !stopped.Unread || stopped.Revision == 0 {
		t.Fatalf("stopped summary: %#v", stopped)
	}
	conversationRequest(
		ctx,
		t,
		backend,
		&laptop,
		"POST",
		"/api/conversations/"+conversationFirst+"/read",
		fmt.Sprintf(`{"revision":%d}`, stopped.Revision),
		204,
	)
	until(func(c webapiproto.ConversationSummary) bool { return !c.Unread })
}

// pageLoginKit supplies the device login result without contacting a vendor.
type pageLoginKit struct{}

// Capability supports login and account addition in the fixture.
func (pageLoginKit) Capability() provider.AccountsCapability {
	return provider.AccountsCapability{Login: true, Add: true}
}

// Login supplies a local device login result.
func (pageLoginKit) Login(ctx context.Context, pending func(types.LoginPending)) (provider.NewAccount, error) {
	code := "ABCD-1234"
	pending(types.LoginPending{VerificationURL: "https://verify.example/device", UserCode: &code})
	if err := ctx.Err(); err != nil {
		return provider.NewAccount{}, err
	}
	identity := "device"
	return provider.NewAccount{
		Secret: `{"token":"login-secret"}`,
		Label:  provider.AccountLabel{Label: "device@example.test", IdentityKey: &identity},
	}, nil
}

// Add reports that fixture account addition is unsupported.
func (pageLoginKit) Add(provider.AddAccount) (provider.NewAccount, error) {
	return provider.NewAccount{}, provider.ErrAccountsUnsupported
}

type pageSubscriptionFamily struct{}

// Credential requires subscription credentials.
func (pageSubscriptionFamily) Credential() webapiproto.CredentialKind {
	return webapiproto.CredentialKindSubscription
}

// Wires declares no external wire APIs.
func (pageSubscriptionFamily) Wires() []types.WireAPI { return nil }

// Provider builds a subscription with local accounts and quota.
func (pageSubscriptionFamily) Provider(args providerhost.FamilyArgs) (provider.Provider, error) {
	subscription, ok := args.Credential.(*providerhost.SubscriptionArgs)
	if !ok {
		return nil, providerhost.ErrWrongCredential
	}
	p := &pageSubscription{
		Provider: openaiapi.New(openaiapi.Config{APIKey: "fixture"}, args.Clock),
		accounts: provider.NewAccounts(subscription.Pool, pageLoginKit{}, args.Clock),
	}
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

// Accounts returns the fixture subscription accounts.
func (p *pageSubscription) Accounts() provider.SubscriptionAccounts { return p.accounts }

// AuthStatus reports the bound account as authenticated.
func (p *pageSubscription) AuthStatus(context.Context) types.AuthState {
	return &types.Authenticated{AccountLabel: p.account}
}

// Quota returns the fixture account quota.
func (p *pageSubscription) Quota() *provider.Quota { return p.quota }

type pageQuota struct{}

// ProbeCost marks the fixture probe as free.
func (pageQuota) ProbeCost() (provider.ProbeCost, bool) {
	return provider.ProbeFree, true
}

// Probe returns the local account quota reading.
func (pageQuota) Probe(context.Context) (provider.ProbeReading, error) {
	percent := float64(40)
	label := "device@example.test"
	return provider.ProbeReading{
		AccountLabel: &label,
		Windows:      []types.QuotaWindow{{ID: "weekly", Label: "Weekly", UsedPercent: &percent}},
	}, nil
}

// Observe produces no quota windows from observations.
func (pageQuota) Observe(provider.Observation) []types.QuotaWindow { return nil }

// TestProviderChangesReachAllUsersWithAccountsOnlyForConfigurer checks that provider changes reach all
// users while accounts remain private to the configurer.
func TestProviderChangesReachAllUsersWithAccountsOnlyForConfigurer(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	harness.Config.Families.Register("device", pageSubscriptionFamily{})
	harness.Config.Families.Register("claude-code", pageSubscriptionFamily{})
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	wireMust(t, harness.AddUser(ctx, "reader@example.test", "reader-pass-1", webapiproto.RoleUser))
	reader, err := backend.Login(ctx, "reader@example.test", "reader-pass-1")
	wireMust(t, err)
	masters, state := conversationPage(ctx, t, backend, &session)
	conversationEqual(t, len(state.Providers), 0)
	readers, _ := conversationPage(ctx, t, backend, &reader)
	started := conversationDecode(
		t,
		conversationRequest(
			ctx,
			t,
			backend,
			&session,
			"POST",
			"/api/providers/subscription-login",
			`{"providerType":"device"}`,
			202,
		),
		webapiproto.DecodeLoginStarted,
	)
	// Publication of the new provider is the event that the login has committed.
	_, err = masters.Until(ctx, func(e webapiproto.SyncEvent) bool {
		p, ok := e.(*webapiproto.SyncEventProviders)
		return ok && len(p.Providers) == 1
	})
	wireMust(t, err)
	login := conversationDecode(
		t,
		conversationRequest(
			ctx,
			t,
			backend,
			&session,
			"GET",
			"/api/providers/subscription-login/"+string(started.Login.ID),
			"",
			200,
		),
		webapiproto.DecodeLoginAnswer,
	)
	completed, ok := login.Login.(*webapiproto.LoginStateCompleted)
	if !ok {
		t.Fatalf("login did not complete: %v", login.Login)
	}
	conversationRequest(
		ctx,
		t,
		backend,
		&session,
		"POST",
		"/api/providers",
		`{"source":"custom","providerType":"anthropic","label":"Work","apiKey":"sk-state"}`,
		201,
	)
	until := func(page *backendtest.SyncChannel) []webapiproto.ProviderState {
		events, err := page.Until(ctx, func(e webapiproto.SyncEvent) bool {
			p, ok := e.(*webapiproto.SyncEventProviders)
			return ok && len(p.Providers) == 2
		})
		wireMust(t, err)
		return events[len(events)-1].(*webapiproto.SyncEventProviders).Providers
	}
	details := func(state webapiproto.ProviderState) webapiproto.ProviderDetails {
		d, ok := state.Details.(*webapiproto.ProviderReadingRead)
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
	if _, ok := details(seen[1]).Auth.(*types.Authenticated); !ok {
		t.Fatal("key entry not authenticated")
	}
	conversationEqual(t, seen[0].ID, completed.ProviderID)
	seen = until(readers)
	subscription = details(seen[0])
	conversationEqual(t, len(subscription.Accounts), 0)
	conversationEqual(t, subscription.Active, (*webapiproto.CredentialID)(nil))
	conversationEqual(t, subscription.Quota, (*types.QuotaSnapshot)(nil))
	conversationEqual[types.AuthState](t, subscription.Auth, &types.Authenticated{})
}
