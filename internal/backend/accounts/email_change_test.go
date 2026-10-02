package accounts

import (
	"context"
	"errors"
	"testing"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/database/databasetest"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

type emailStore struct {
	EmailStore
	account *database.Account
	taken   bool
}

func (s *emailStore) Account(context.Context, webapi.UserID) (*database.Account, error) {
	return s.account, nil
}
func (s *emailStore) EmailInUse(context.Context, webapi.EmailAddress) (bool, error) {
	return s.taken, nil
}

type unexpectedMail struct{ t *testing.T }

func (m unexpectedMail) SendVerification(context.Context, VerificationMail) error {
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
		want    StartRefusal
	}{
		{name: "delivery disabled", want: MailUnavailable},
		{name: "account absent", mail: true, want: InvalidCredentials},
		{name: "wrong password", mail: true, account: true, want: InvalidCredentials},
		{name: "address taken", mail: true, account: true, valid: true, taken: true, want: EmailTaken},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			store := &emailStore{taken: tc.taken}
			if tc.account {
				store.account = &database.Account{}
			}
			passwords := &testPasswords{valid: tc.valid}
			var mail AccountMail
			if tc.mail {
				mail = unexpectedMail{t: t}
			}
			outcome, err := NewEmailChanges(store, passwords, mail, CodeKey{}).Start(t.Context(), "caller", "new@example.test", "password")
			if err != nil {
				t.Fatal(err)
			}
			refused, ok := outcome.(*StartRefused)
			if !ok || refused.Reason != tc.want {
				t.Fatalf("got %#v", outcome)
			}
			if !tc.account && passwords.verified != 0 {
				t.Fatal("absent account verified password")
			}
		})
	}
}

// capturedMail observes delivery while all account and challenge state stays in SQLite.
type capturedMail struct {
	messages []VerificationMail
	failure  error
	cancel   context.CancelFunc
}

func (m *capturedMail) SendVerification(ctx context.Context, mail VerificationMail) error {
	m.messages = append(m.messages, mail)
	if m.cancel != nil {
		m.cancel()
		return ctx.Err()
	}
	return m.failure
}

func TestEmailDeliveryConfirmationAndFailureCleanup(t *testing.T) {
	control := databasetest.Control(t.Context(), t, &accountClock{at: core.UnixEpoch})
	h, err := NewPasswordHasher(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	accounts := NewAccounts(control, h, NewWebSessions(control), NewLoginLimiter())
	signed, err := accounts.Setup(t.Context(), webapi.SetupRequest{Email: "old@example.test", Password: "password123"})
	if err != nil {
		t.Fatal(err)
	}
	mail := &capturedMail{}
	changes := NewEmailChanges(control, h, mail, NewCodeKey(databasetest.Key()))
	outcome, err := changes.Start(t.Context(), signed.User.ID, "new@example.test", "password123")
	if err != nil {
		t.Fatal(err)
	}
	issued, ok := outcome.(*StartIssued)
	if !ok || len(mail.messages) != 1 {
		t.Fatalf("issue and delivery: %#v, %d messages", outcome, len(mail.messages))
	}
	delivered := mail.messages[0]
	if delivered.Email != issued.Challenge.Email || delivered.ExpiresAt != issued.Challenge.ExpiresAt {
		t.Fatal("delivered challenge differs from response")
	}
	outcome, err = changes.Start(t.Context(), signed.User.ID, "new@example.test", "password123")
	refused, ok := outcome.(*StartRefused)
	if err != nil || !ok || refused.Reason != CoolingDown || len(mail.messages) != 1 {
		t.Fatalf("cooldown: %#v, %v", outcome, err)
	}
	confirmation, err := changes.Confirm(t.Context(), signed.User.ID, issued.Challenge.ID, delivered.Code+"x")
	if _, ok := confirmation.(*database.ChallengeInvalidCode); err != nil || !ok {
		t.Fatalf("wrong code: %#v, %v", confirmation, err)
	}
	confirmation, err = changes.Confirm(t.Context(), signed.User.ID, issued.Challenge.ID, delivered.Code)
	changed, ok := confirmation.(*database.ChallengeChanged)
	if err != nil || !ok || changed.User.Email != delivered.Email {
		t.Fatalf("confirm: %#v, %v", confirmation, err)
	}
	stored, err := control.AccountByEmail(t.Context(), delivered.Email)
	if err != nil || stored == nil || stored.User.ID != signed.User.ID {
		t.Fatalf("new address not persisted: %v", err)
	}
	old, err := control.AccountByEmail(t.Context(), "old@example.test")
	if err != nil || old != nil {
		t.Fatalf("old address still resolves: %v", err)
	}
	confirmation, err = changes.Confirm(t.Context(), signed.User.ID, issued.Challenge.ID, delivered.Code)
	if _, ok := confirmation.(*database.ChallengeInvalidCode); err != nil || !ok {
		t.Fatalf("consumed code reused: %#v, %v", confirmation, err)
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
			outcome, err := changes.Start(ctx, signed.User.ID, email, "password123")
			refused, ok := outcome.(*StartRefused)
			if err != nil || !ok || refused.Reason != MailFailed {
				t.Fatalf("failed delivery: %#v, %v", outcome, err)
			}
			mail.failure = nil
			mail.cancel = nil
			// Retry immediately, without advancing the clock: failed delivery must
			// delete the challenge and release its one-minute cooldown.
			outcome, err = changes.Start(t.Context(), signed.User.ID, email, "password123")
			retried, ok := outcome.(*StartIssued)
			if err != nil || !ok {
				t.Fatalf("failure retained cooldown: %#v, %v", outcome, err)
			}
			latest := mail.messages[len(mail.messages)-1]
			confirmation, err := changes.Confirm(t.Context(), signed.User.ID, retried.Challenge.ID, latest.Code)
			if _, ok := confirmation.(*database.ChallengeChanged); err != nil || !ok {
				t.Fatalf("retry confirmation: %#v, %v", confirmation, err)
			}
		})
	}
}
