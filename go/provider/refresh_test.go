package provider_test

import (
	"context"
	"errors"
	"sync"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/go/gates"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/provider/internal/testfixture"
)

// document is a test account store, not an account metadata or pool substitute.
// It isolates the public versioned-document refresh boundary.
type document struct {
	mu       sync.Mutex
	revision *provider.Revision
	gates    *provider.RefreshGates
}

func newDocument(t *testing.T) *document {
	t.Helper()
	return &document{revision: &provider.Revision{Text: `{"access":"old","refresh":"r1"}`, Version: 1}, gates: provider.NewRefreshGates()}
}
func (d *document) Name() string { return "test account" }
func (d *document) Read(ctx context.Context) (*provider.Revision, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	d.mu.Lock()
	defer d.mu.Unlock()
	if d.revision == nil {
		return nil, nil
	}
	copy := *d.revision
	return &copy, nil
}
func (d *document) Replace(ctx context.Context, text string, version uint64) (bool, error) {
	if err := ctx.Err(); err != nil {
		return false, err
	}
	d.mu.Lock()
	defer d.mu.Unlock()
	if d.revision == nil || d.revision.Version != version {
		return false, nil
	}
	d.revision = &provider.Revision{Text: text, Version: version + 1}
	return true, nil
}
func (d *document) RefreshTurn(ctx context.Context) (*gates.KeyedPermit[string], error) {
	return d.gates.Acquire(ctx, "entry/account")
}
func decodeTokens(data []byte) (testfixture.Tokens, error) {
	return provider.DecodeSecret(data, testfixture.DecodeTokens)
}
func renew(ctx context.Context, d *document, due func(testfixture.Tokens) bool, refresh func(context.Context, testfixture.Tokens) (testfixture.Tokens, error)) (testfixture.Tokens, error) {
	return provider.Renew(ctx, d, decodeTokens, testfixture.EncodeTokens, due, refresh)
}
func always(testfixture.Tokens) bool    { return true }
func expired(s testfixture.Tokens) bool { return s.Access == "old" }

func TestRenewSerializesAndRechecksDueAfterWaiting(t *testing.T) {
	for _, stillDue := range []bool{false, true} {
		synctest.Test(t, func(t *testing.T) {
			d := newDocument(t)
			entered := make(chan struct{})
			finish := make(chan struct{})
			results := make(chan testfixture.Tokens, 2)
			failures := make(chan error, 2)
			var spent []string
			refresh := func(ctx context.Context, s testfixture.Tokens) (testfixture.Tokens, error) {
				spent = append(spent, s.Refresh)
				if len(spent) == 1 {
					close(entered)
					<-finish
				}
				return testfixture.Tokens{Access: "new", Refresh: "after-" + s.Refresh}, nil
			}
			due := expired
			if stillDue {
				due = always
			}
			run := func() { got, err := renew(t.Context(), d, due, refresh); results <- got; failures <- err }
			go run()
			<-entered
			go run()
			synctest.Wait()
			close(finish)
			for range 2 {
				if err := <-failures; err != nil {
					t.Fatal(err)
				}
				if got := <-results; got.Access != "new" {
					t.Fatalf("secret=%+v", got)
				}
			}
			wantCalls := 1
			if stillDue {
				wantCalls = 2
			}
			if len(spent) != wantCalls || spent[0] != "r1" {
				t.Fatalf("spent=%v", spent)
			}
			if stillDue && spent[1] != "after-r1" {
				t.Fatalf("spent stale token: %v", spent)
			}
			revision, err := d.Read(t.Context())
			if err != nil || revision.Version != uint64(1+wantCalls) {
				t.Fatalf("revision=%+v err=%v", revision, err)
			}
		})
	}
}
func TestRenewYieldsToCompetingWriterOnSuccessAndRefusal(t *testing.T) {
	refused := errors.New("refresh token revoked")
	for _, refusal := range []bool{false, true} {
		d := newDocument(t)
		got, err := renew(t.Context(), d, always, func(ctx context.Context, s testfixture.Tokens) (testfixture.Tokens, error) {
			kept, err := d.Replace(ctx, `{"access":"winner","refresh":"rw"}`, 1)
			if err != nil || !kept {
				t.Fatal("competing write failed", err)
			}
			if refusal {
				return testfixture.Tokens{}, refused
			}
			return testfixture.Tokens{Access: "loser", Refresh: "rl"}, nil
		})
		if err != nil || got.Access != "winner" {
			t.Fatalf("got=%+v err=%v", got, err)
		}
		got, err = renew(t.Context(), d, always, func(context.Context, testfixture.Tokens) (testfixture.Tokens, error) {
			return testfixture.Tokens{}, refused
		})
		if !errors.Is(err, refused) {
			t.Fatalf("lost refusal: %v", err)
		}
	}
}
func TestRenewSkipsCurrentSecretAndRefusesMissingOrCorrupt(t *testing.T) {
	d := newDocument(t)
	refresh := func(context.Context, testfixture.Tokens) (testfixture.Tokens, error) {
		t.Fatal("unexpected refresh")
		return testfixture.Tokens{}, nil
	}
	got, err := renew(t.Context(), d, func(testfixture.Tokens) bool { return false }, refresh)
	if err != nil || got.Access != "old" {
		t.Fatalf("got=%+v err=%v", got, err)
	}
	d.revision = nil
	if _, err := renew(t.Context(), d, always, refresh); !errors.Is(err, provider.ErrMissingDocument) {
		t.Fatalf("missing: %v", err)
	}
	d.revision = &provider.Revision{Text: `{"access":"sk-secret","refresh":7}`, Version: 1}
	if _, err := renew(t.Context(), d, always, refresh); err == nil {
		t.Fatal("corrupt document accepted")
	}
}
func TestRenewCancellationReleasesTurnWithoutPublishing(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		d := newDocument(t)
		ctx, cancel := context.WithCancel(t.Context())
		entered := make(chan struct{})
		done := make(chan error, 1)
		go func() {
			_, err := renew(ctx, d, always, func(ctx context.Context, s testfixture.Tokens) (testfixture.Tokens, error) {
				close(entered)
				<-ctx.Done()
				return testfixture.Tokens{Access: "cancelled", Refresh: "bad"}, nil
			})
			done <- err
		}()
		<-entered
		cancel()
		if err := <-done; !errors.Is(err, context.Canceled) {
			t.Fatalf("cancel: %v", err)
		}
		got, err := renew(t.Context(), d, always, func(ctx context.Context, s testfixture.Tokens) (testfixture.Tokens, error) {
			return testfixture.Tokens{Access: "new", Refresh: s.Refresh}, nil
		})
		if err != nil || got.Refresh != "r1" {
			t.Fatalf("cancel published or held turn: %+v %v", got, err)
		}
	})
}
func TestCredentialIDUsesIdentityThenLabel(t *testing.T) {
	if got := provider.CredentialIDFor("acct-1", "different"); got != "cred-ba36a4edd92d37c6" {
		t.Fatal(got)
	}
	if provider.CredentialIDFor("", "acct-1") != provider.CredentialIDFor("acct-1", "") {
		t.Fatal("label fallback differs")
	}
}
