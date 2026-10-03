package backend_test

import (
	"fmt"
	"testing"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/webapi"
)

// TestUsageGroupsInFirstUseOrderAndInstanceRequiresAdministrator checks that usage groups retain
// first-use order and instance usage requires an administrator.
// Ledger rows are seeded directly; this scenario tests the HTTP view.
func TestUsageGroupsInFirstUseOrderAndInstanceRequiresAdministrator(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	backend, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	wireMust(t, harness.AddUser(ctx, "admin@example.test", "admin-pass-1", webapi.RoleAdmin))
	wireMust(t, harness.AddUser(ctx, "bob@example.test", "bob-pass-1", webapi.RoleUser))
	db, err := harness.ControlDatabase(ctx, t)
	wireMust(t, err)
	for i, row := range []struct {
		email, provider, model string
		input                  int
	}{
		{backendtest.MasterEmail, "entry-b", "model-2", 100},
		{backendtest.MasterEmail, "entry-a", "model-1", 200},
		{backendtest.MasterEmail, "entry-b", "model-2", 300},
		{"bob@example.test", "entry-a", "model-1", 50},
	} {
		_, err := db.ExecContext(
			ctx,
			`INSERT INTO usage_ledger (id,user_id,conversation_id,provider_id,model_id,`+
				`input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,created_at) `+
				`SELECT ?,id,?,?,?,?,10,2,1,? FROM users WHERE email=?`,
			fmt.Sprintf("usage-%d", i),
			conversationFirst,
			row.provider,
			row.model,
			row.input,
			int64(1790000000000)+int64(i+1),
			row.email,
		)
		wireMust(t, err)
	}
	own := conversationDecode(
		t,
		conversationRequest(ctx, t, backend, &master, "GET", "/api/usage", "", 200),
		webapi.DecodeUsageTotals,
	)
	type group struct {
		Provider, Model         string
		Requests, Input, Output uint64
	}
	var groups []group
	for _, g := range own.Totals {
		groups = append(
			groups,
			group{string(g.ProviderID), string(g.ModelID), g.Requests, g.InputTokens, g.OutputTokens},
		)
	}
	conversationEqual(t, groups, []group{{"entry-b", "model-2", 2, 400, 20}, {"entry-a", "model-1", 1, 200, 10}})
	conversationEqual(t, own.Totals[0].CacheReadTokens, uint64(4))
	conversationEqual(t, own.Totals[0].CacheWriteTokens, uint64(2))
	bob, err := backend.Login(ctx, "bob@example.test", "bob-pass-1")
	wireMust(t, err)
	theirs := conversationDecode(
		t,
		conversationRequest(ctx, t, backend, &bob, "GET", "/api/usage", "", 200),
		webapi.DecodeUsageTotals,
	)
	conversationEqual(t, len(theirs.Totals), 1)
	conversationRefusal(
		t,
		conversationRequest(ctx, t, backend, &bob, "GET", "/api/usage/instance", "", 403),
		webapi.ErrorCodeForbidden,
	)
	admin, err := backend.Login(ctx, "admin@example.test", "admin-pass-1")
	wireMust(t, err)
	instance := conversationDecode(
		t,
		conversationRequest(ctx, t, backend, &admin, "GET", "/api/usage/instance", "", 200),
		webapi.DecodeInstanceUsage,
	)
	type user struct {
		Email    string
		Requests uint64
	}
	var users []user
	for _, u := range instance.Users {
		count := uint64(0)
		for _, g := range u.Totals {
			count += g.Requests
		}
		users = append(users, user{string(u.Email), count})
	}
	conversationEqual(
		t,
		users,
		[]user{{backendtest.MasterEmail, 3}, {"admin@example.test", 0}, {"bob@example.test", 1}},
	)
	wireMust(t, backend.Close(ctx))
	backend, err = harness.StartInMode(ctx, t, webapi.InstanceModeIsolated)
	wireMust(t, err)
	master, err = backend.Login(ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	conversationRefusal(
		t,
		conversationRequest(ctx, t, backend, &master, "GET", "/api/usage/instance", "", 403),
		webapi.ErrorCodeForbidden,
	)
}
