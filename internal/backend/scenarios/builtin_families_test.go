package scenarios_test

import (
	"net/http"
	"testing"

	"github.com/wspl/demi/internal/backend"
	"github.com/wspl/demi/internal/backend/providerhost"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/types"
)

// TestTheSubscriptionFamiliesLogInByDeviceAndStandForTheirBoundAccount
// checks subscription login and account binding.
func TestTheSubscriptionFamiliesLogInByDeviceAndStandForTheirBoundAccount(t *testing.T) {
	t.Parallel()
	ctx := t.Context()
	registry := backend.BuiltinFamilies()
	conversationEqual(t, registry.Subscriptions(), []string{"claude-code", "codex", "grok-build"})
	const now types.Timestamp = "2026-09-18T14:00:00.000Z"
	clock := providertest.NewManualClock(now)
	endpoint := provider.ModelsDevURL
	codex := accountJSON(t,
		contract.Field{Name: "accessToken", Value: providertest.JWT(t, struct {
			Exp int64 `json:"exp"`
		}{1900000000})},
		contract.Field{Name: "refreshToken", Value: "refresh-1"},
		contract.Field{Name: "idToken", Value: providertest.JWT(t, struct {
			Email string `json:"email"`
		}{"user@example.com"})},
		contract.Field{Name: "accountId", Value: "acct-1"},
		contract.Field{Name: "lastRefresh", Value: now},
	)
	grok := `{"accessToken":"session-token","issuer":"https://auth.x.ai","clientId":"client-1",` +
		`"email":"user@example.com"}`
	for _, row := range []struct{ family, secret, name string }{{"codex", codex, "Codex"}, {"grok-build", grok, "Grok"}} {
		build := func(pool *provider.MemoryCredentialPool, binding *providerhost.AccountBinding) provider.Provider {
			p, err := registry.Family(row.family).Provider(providerhost.FamilyArgs{
				EntryID:    "entry-1",
				Label:      row.family,
				Credential: &providerhost.SubscriptionArgs{Pool: pool, Account: binding},
				HTTP:       http.DefaultClient,
				Clock:      clock,
				ModelsDev:  provider.NewModelsDevClient(http.DefaultClient, endpoint, clock),
			})
			if err != nil {
				t.Fatal(err)
			}
			return p
		}
		login := build(provider.NewMemoryCredentialPool(), nil)
		conversationEqual(t, login.Accounts().Capability(), provider.AccountsCapability{Login: true})
		message := "No " + row.name + " account is signed in"
		conversationEqual[types.AuthState](t, login.AuthStatus(ctx), &types.Unauthenticated{Message: &message})
		pool := provider.NewMemoryCredentialPool()
		if err := pool.Write(
			ctx,
			provider.AccountMeta{ID: "cred-1", Label: "user@example.com", UpdatedAt: now, Source: "login:device"},
			row.secret,
		); err != nil {
			t.Fatal(err)
		}
		quota := &provider.MemorySnapshots{}
		label := "user@example.com"
		quota.Update(func(*types.QuotaSnapshot) types.QuotaSnapshot {
			return types.QuotaSnapshot{
				ObservedAt:   now,
				Source:       types.SnapshotSourceProbe,
				AccountLabel: &label,
				Windows:      []types.QuotaWindow{},
			}
		})
		p := build(pool, &providerhost.AccountBinding{CredentialID: "cred-1", Quota: quota})
		conversationEqual[types.AuthState](t, p.AuthStatus(ctx), &types.Authenticated{AccountLabel: &label})
		conversationEqual(t, p.Quota().Latest(), quota.Latest())
	}
}
