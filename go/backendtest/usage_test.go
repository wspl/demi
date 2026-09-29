package backendtest_test

import (
	"net/http"
	"testing"

	"github.com/wspl/demi/go/backendtest"
	"github.com/wspl/demi/go/backendtest/scripted"
)

// meteredAnswer is an answer whose request the ledger records with the tokens:
// input, ten of output, two read from the cache and one written to it.
func meteredAnswer(input int) *scripted.Response {
	usage := scripted.Frame{"input_tokens": input, "output_tokens": 0, "cache_read_input_tokens": 2, "cache_creation_input_tokens": 1}
	return scripted.Message([][]scripted.Frame{scripted.TextBlock(0, []string{"ok"})}, "end_turn", usage, 10)
}

// The ledger's totals (usage-and-quota.md § Usage ledger): one group per entry
// and model in the order each pair was first used, each user's own, and on a
// shared instance every account's for an administrator. The metered runtime that
// writes the rows is the usage module's own test; here the rows are written by
// real requests to a scripted vendor, since a black box has no ledger to write
// to.
//
// Cost: one backend started twice and a scripted vendor, about two seconds.
func TestTotalsGroupByEntryAndModelInFirstUseOrderAndTheInstanceViewIsForAdministrators(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	h := backendtest.New(t)
	b, master := h.StartSetUp()
	b.CreateUser(master, "admin@example.test", "admin-pass-1", "admin")
	bob := b.CreateUser(master, "bob@example.test", "bob-pass-1", "user")
	entry := func(prefix, model string) string {
		return newEntry(b, master, backendtest.Map{
			"source": "custom", "providerType": "anthropic", "label": model, "apiKey": "sk-ant-test",
			"baseUrl": vendor.URL(prefix + "/v1"),
			"models": []any{backendtest.Map{
				"id": model, "displayName": model, "contextWindow": 100000, "outputLimit": nil,
				"thinkingEfforts": []any{}, "acceptedExtensions": nil, "fastTier": nil,
			}},
		})
	}
	entryA, entryB := entry("/a", "model-1"), entry("/b", "model-2")
	turn := func(session *backendtest.Session, conversation, provider, model, prefix string, input int) {
		t.Helper()
		b.CreateConversation(session, conversation)
		b.Choose(session, conversation, provider, model)
		socket := b.Connect(session, conversation)
		socket.Open()
		vendor.RespondAt(prefix+"/v1/messages", meteredAnswer(input))
		socket.Chat("m1", "hello")
		socket.Close()
	}
	// The master uses entry b first, then entry a, then entry b again; bob uses
	// entry a.
	turn(master, convFirst, entryB, "model-2", "/b", 100)
	turn(master, convSecond, entryA, "model-1", "/a", 200)
	vendor.RespondAt("/b/v1/messages", meteredAnswer(300))
	again := b.Connect(master, convFirst)
	again.Open()
	again.Chat("m2", "again")
	again.Close()
	turn(bob, convThird, entryA, "model-1", "/a", 50)

	type group struct {
		provider, model string
		requests        float64
		input, output   float64
	}
	groups := func(totals []any) []group {
		var list []group
		for _, total := range totals {
			list = append(list, group{
				backendtest.At(total, "providerId").(string), backendtest.At(total, "modelId").(string),
				backendtest.At(total, "requests").(float64), backendtest.At(total, "inputTokens").(float64), backendtest.At(total, "outputTokens").(float64),
			})
		}
		return list
	}
	own := usageTotals(b, master)
	want := []group{{entryB, "model-2", 2, 400, 20}, {entryA, "model-1", 1, 200, 10}}
	if got := groups(own); len(got) != 2 || got[0] != want[0] || got[1] != want[1] {
		t.Fatalf("the master's totals are %v, not %v", got, want)
	}
	if backendtest.At(own[0], "cacheReadTokens") != 4.0 || backendtest.At(own[0], "cacheWriteTokens") != 2.0 {
		t.Fatalf("the cache tokens are %v", own[0])
	}

	if bobs := usageTotals(b, bob); len(bobs) != 1 {
		t.Fatalf("bob's totals are %v", bobs)
	}
	wantRefusal(t, b.Get("/api/usage/instance", bob), http.StatusForbidden, "forbidden", "the instance view for a user")
	admin := b.Login("admin@example.test", "admin-pass-1")
	instance := b.Get("/api/usage/instance", admin).Expect(http.StatusOK)
	users, _ := instance.At("users").([]any)
	type count struct {
		email    string
		requests float64
	}
	var counts []count
	for _, user := range users {
		requests := 0.0
		totals, _ := backendtest.At(user, "totals").([]any)
		for _, total := range totals {
			requests += backendtest.At(total, "requests").(float64)
		}
		counts = append(counts, count{backendtest.At(user, "email").(string), requests})
	}
	wantCounts := []count{{"master@example.test", 3}, {"admin@example.test", 0}, {"bob@example.test", 1}}
	if len(counts) != 3 || counts[0] != wantCounts[0] || counts[1] != wantCounts[1] || counts[2] != wantCounts[2] {
		t.Fatalf("the instance's requests are %v, not %v", counts, wantCounts)
	}
	b.Stop()

	// An isolated instance has no instance view.
	h.SetMode("isolated")
	b = h.Start()
	master = b.Login(backendtest.MasterEmail, backendtest.MasterPassword)
	wantRefusal(t, b.Get("/api/usage/instance", master), http.StatusForbidden, "forbidden", "the instance view of an isolated instance")
	b.Stop()
}
