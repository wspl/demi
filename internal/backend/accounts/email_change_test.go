package accounts_test

import (
	"context"
	"errors"
	"testing"

	"github.com/wspl/demi/internal/backend/accounts"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/database/databasetest"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

type emailStore struct {
	accounts.EmailStore
	account *database.Account
	taken   bool
}

func (s *emailStore) Account(context.Context, webapi.UserID) (database.Account, bool, error) {
	if s.account == nil {
		return database.Account{}, false, nil
	}
	return *s.account, true, nil
}

func (s *emailStore) EmailInUse(context.Context, webapi.EmailAddress) (bool, error) {
	return s.taken, nil
}

type unexpectedMail struct{ t *testing.T }

func (m unexpectedMail) SendVerification(context.Context, accounts.VerificationMail) error {
	m.t.Fatal("unexpected mail")
	return nil
}

func TestEmailStartRefusalsDoNotIssueChallenges(t *testing.T) {
	cases := []struct {
		name    string
		mail    bool
		account bool
		valid   bool
		taken   bool
		want    error
	}{
		{name: "delivery disabled", want: accounts.ErrMailUnavailable},
		{name: "account absent", mail: true, want: accounts.ErrCurrentPassword},
		{name: "wrong password", mail: true, account: true, want: accounts.ErrCurrentPassword},
		{name: "address taken", mail: true, account: true, valid: true, taken: true, want: database.ErrEmailTaken},
	}
	for _, scenario := range cases {
		t.Run(scenario.name, func(t *testing.T) {
			store := &emailStore{taken: scenario.taken}
			if scenario.account {
				store.account = &database.Account{}
			}
			passwords := &testPasswords{valid: scenario.valid}
			var mail accounts.AccountMail
			if scenario.mail {
				mail = unexpectedMail{t: t}
			}
			_, err := accounts.NewEmailChanges(
				store,
				passwords,
				mail,
				accounts.CodeKey{},
			).Start(t.Context(), "caller", "new@example.test", "password")
			if !errors.Is(err, scenario.want) {
				t.Fatalf("got %v", err)
			}
			if !scenario.account && passwords.verified != 0 {
				t.Fatal("absent account verified password")
			}
		})
	}
}

// capturedMail observes delivery while all account and challenge state stays in SQLite.
type capturedMail struct {
	messages []accounts.VerificationMail
	failure  error
	cancel   context.CancelFunc
}

func (m *capturedMail) SendVerification(ctx context.Context, mail accounts.VerificationMail) error {
	m.messages = append(m.messages, mail)
	if m.cancel != nil {
		m.cancel()
		return ctx.Err()
	}
	return m.failure
}

func TestEmailDeliveryConfirmationAndFailureCleanup(t *testing.T) {
	control := databasetest.Control(t.Context(), t, &accountClock{at: core.UnixEpoch})
	h, err := accounts.NewPasswordHasher(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	service := accounts.New(control, h, accounts.NewWebSessions(control), accounts.NewLoginLimiter())
	signed, err := service.Setup(t.Context(), webapi.SetupRequest{Email: "old@example.test", Password: "password123"})
	if err != nil {
		t.Fatal(err)
	}
	mail := &capturedMail{}
	changes := accounts.NewEmailChanges(control, h, mail, accounts.NewCodeKey(databasetest.Key()))
	challenge, err := changes.Start(t.Context(), signed.User.ID, "new@example.test", "password123")
	if err != nil || len(mail.messages) != 1 {
		t.Fatalf("issue and delivery: %v, %d messages", err, len(mail.messages))
	}
	delivered := mail.messages[0]
	if delivered.Email != challenge.Email || delivered.ExpiresAt != challenge.ExpiresAt {
		t.Fatal("delivered challenge differs from response")
	}
	_, err = changes.Start(t.Context(), signed.User.ID, "new@example.test", "password123")
	if !errors.Is(err, database.ErrCoolingDown) || len(mail.messages) != 1 {
		t.Fatalf("cooldown: %v", err)
	}
	_, err = changes.Confirm(t.Context(), signed.User.ID, challenge.ID, delivered.Code+"x")
	if !errors.Is(err, database.ErrInvalidCode) {
		t.Fatalf("wrong code: %v", err)
	}
	changed, err := changes.Confirm(t.Context(), signed.User.ID, challenge.ID, delivered.Code)
	if err != nil || changed.Email != delivered.Email {
		t.Fatalf("confirm: %#v, %v", changed, err)
	}
	stored, found, err := control.AccountByEmail(t.Context(), delivered.Email)
	if err != nil || !found || stored.User.ID != signed.User.ID {
		t.Fatalf("new address not persisted: %v", err)
	}
	_, ok, err := control.AccountByEmail(t.Context(), "old@example.test")
	if err != nil || ok {
		t.Fatalf("old address still resolves: %v", err)
	}
	_, err = changes.Confirm(t.Context(), signed.User.ID, challenge.ID, delivered.Code)
	if !errors.Is(err, database.ErrInvalidCode) {
		t.Fatalf("consumed code reused: %v", err)
	}

	for _, canceled := range []bool{false, true} {
		name := "delivery error"
		email := webapi.EmailAddress("retry@example.test")
		if canceled {
			name = "canceled delivery"
			email = "retry-canceled@example.test"
		}
		t.Run(name, func(t *testing.T) {
			ctx, cancel := context.WithCancel(t.Context())
			defer cancel()
			mail.failure = errors.New("delivery failed")
			if canceled {
				mail.cancel = cancel
			}
			_, err := changes.Start(ctx, signed.User.ID, email, "password123")
			if !errors.Is(err, accounts.ErrMailFailed) {
				t.Fatalf("failed delivery: %v", err)
			}
			mail.failure = nil
			mail.cancel = nil
			// Retry immediately, without advancing the clock: failed delivery must
			// delete the challenge and release its one-minute cooldown.
			retried, err := changes.Start(t.Context(), signed.User.ID, email, "password123")
			if err != nil {
				t.Fatalf("failure retained cooldown: %v", err)
			}
			latest := mail.messages[len(mail.messages)-1]
			if _, err := changes.Confirm(t.Context(), signed.User.ID, retried.ID, latest.Code); err != nil {
				t.Fatalf("retry confirmation: %v", err)
			}
		})
	}
}
