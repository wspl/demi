package usage

import (
	"iter"
	"path/filepath"
	"testing"
	"time"

	"github.com/wspl/demi/go/backend/storage"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/webapi"
)

type clock struct{ now core.Timestamp }

func (c *clock) Now() core.Timestamp { return c.now }

type event struct {
	tokens  *core.TokenUsage
	failure *RateLimited
}

// Cost: one small SQLite file, four scripted requests, no network or real waits.
func TestResponseIsRecordedBeforeDeliveryAndRejectedRunNeverStarts(t *testing.T) {
	ctx := t.Context()
	clock := &clock{core.TruncateTimestamp(time.UnixMilli(1700000000000))}
	c, err := storage.OpenControl(ctx, filepath.Join(t.TempDir(), "control.sqlite"), clock)
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	email, _ := webapi.ParseEmailAddress("fixture@example.test")
	hash, _ := storage.ParsePasswordHash("$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHRzYWx0$0mUbQTTMhhaEBFGMq7WTZxOlVoS9sY3qVqLiV7Q1Izo")
	user, err := c.CreateMaster(ctx, email, hash)
	if err != nil {
		t.Fatal(err)
	}
	conversation, _ := webapi.ParseConversationID("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01")
	provider, _ := webapi.ParseProviderID("entry")
	meter := &Meter{RateLimitWithLimit(clock, 3), Ledger{c, user.ID, conversation, provider}}
	starts := 0
	run := func() iter.Seq[event] {
		starts++
		tokens := core.TokenUsage{InputTokens: uint64(99 + starts), OutputTokens: 10}
		return func(yield func(event) bool) { yield(event{tokens: &tokens}) }
	}
	response := func(e event) (core.TokenUsage, bool) {
		if e.tokens == nil {
			return core.TokenUsage{}, false
		}
		return *e.tokens, true
	}
	refused := func(e *RateLimited) event { return event{failure: e} }
	for i := 1; i <= 4; i++ {
		for e := range MeterEvents(ctx, meter, "model", run, response, refused) {
			totals, err := c.UsageTotals(ctx, user.ID)
			if err != nil {
				t.Fatal(err)
			}
			if totals[0].Requests != uint64(min(i, 3)) {
				t.Fatalf("response overtook ledger: %+v", totals)
			}
			if i == 4 && (e.failure == nil || e.failure.Error() != "Provider request rate limit reached (3 per minute)") {
				t.Fatalf("refusal: %+v", e)
			}
		}
	}
	if starts != 3 {
		t.Fatalf("rejected request reached provider: %d", starts)
	}
	totals, err := c.UsageTotals(ctx, user.ID)
	if err != nil || totals[0].InputTokens != 303 || totals[0].OutputTokens != 30 {
		t.Fatalf("totals: %+v %v", totals, err)
	}
	clock.now = core.TruncateTimestamp(clock.now.Time().Add(time.Minute))
	for e := range MeterEvents(ctx, meter, "model", run, response, refused) {
		if e.failure != nil {
			t.Fatal("window did not expire")
		}
	}
}
