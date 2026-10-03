package providers

import (
	"context"
	"reflect"
	"sync"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

func TestRequestsAdmittedAsEarliestLeaveWindow(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		limit := NewRequestRateLimit(3)
		for range 2 {
			if err := limit.Take(); err != nil {
				t.Fatal(err)
			}
		}
		time.Sleep(30 * time.Second)
		if err := limit.Take(); err != nil {
			t.Fatal(err)
		}
		if err := limit.Take(); err == nil || err.Error() != "Provider request rate limit reached (3 per minute)" {
			t.Fatal(err)
		}
		time.Sleep(30 * time.Second)
		for range 2 {
			if err := limit.Take(); err != nil {
				t.Fatal(err)
			}
		}
		if err := limit.Take(); err == nil || err.Error() != "Provider request rate limit reached (3 per minute)" {
			t.Fatal(err)
		}
	})
}

// SQLite is real IO, so this scenario uses event synchronization rather than virtual time.
func TestResponsesReachLedgerBeforeAgentAndLimitRefusesRest(t *testing.T) {
	ctx := t.Context()
	vault, owner := vaultFixture(t)
	script := providertest.NewScriptedRuntime(t,
		providertest.Events(providertest.Text("a"), providertest.Response(100, 10)),
		providertest.Events(providertest.Response(101, 10)),
		providertest.Events(providertest.Response(102, 10)),
		providertest.Events(providertest.Response(103, 10)),
	)
	first := NewMeteredRuntime(
		script,
		NewRequestRateLimit(3),
		Ledger{
			Control:      vault.control,
			User:         owner.ID,
			Conversation: "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b",
			Provider:     "entry-1",
		},
	)
	defer func() {
		if err := first.Close(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	var firstEvents []provider.Event
	for event := range first.Run(ctx, providertest.InferenceRequest()) {
		firstEvents = append(firstEvents, event)
		totals, err := vault.control.UsageTotals(ctx, owner.ID)
		if err != nil {
			t.Fatal(err)
		}
		if _, ok := event.(*provider.Response); ok {
			if len(totals) != 1 || totals[0].Requests != 1 {
				t.Fatalf("response preceded ledger: %v", totals)
			}
		} else if len(totals) != 0 {
			t.Fatal("ledger before response", totals)
		}
	}
	if want := []provider.Event{
		providertest.Text("a"),
		providertest.Response(100, 10),
	}; !reflect.DeepEqual(
		firstEvents,
		want,
	) {
		t.Fatalf("first run: %#v; want %#v", firstEvents, want)
	}
	var workers sync.WaitGroup
	outcomes := make([][]provider.Event, 3)
	for i := range 3 {
		fork := first.Fresh()
		workers.Go(func() {
			defer func() {
				if err := fork.Close(context.Background()); err != nil {
					t.Error(err)
				}
			}()
			for event := range fork.Run(ctx, providertest.InferenceRequest()) {
				outcomes[i] = append(outcomes[i], event)
			}
		})
	}
	workers.Wait()
	refused := 0
	for _, events := range outcomes {
		if len(events) == 1 {
			if e, ok := events[0].(*provider.Error); ok && e.Failure.Code != nil &&
				*e.Failure.Code == provider.ErrorCode("rate_limited") {
				refused++
			}
		}
	}
	if refused != 1 || script.Remaining() != 1 {
		t.Fatalf("refusals %d, unused turns %d", refused, script.Remaining())
	}
	totals, err := vault.control.UsageTotals(ctx, owner.ID)
	if err != nil {
		t.Fatal(err)
	}
	if len(totals) != 1 || totals[0].Requests != 3 || totals[0].InputTokens != 303 || totals[0].OutputTokens != 30 ||
		totals[0].ProviderID != "entry-1" ||
		totals[0].ModelID != "model-1" {
		t.Fatalf("totals: %+v", totals)
	}
}
