package accounts

import (
	"context"
	"testing"

	"github.com/wspl/demi/internal/backend/database"
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
