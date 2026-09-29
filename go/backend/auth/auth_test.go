package auth

import (
	"context"
	"errors"
	"path/filepath"
	"testing"
	"time"

	"github.com/wspl/demi/go/backend/storage"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/webapi"
)

type clock struct{ at core.Timestamp }

func (c *clock) Now() core.Timestamp     { return c.at }
func (c *clock) advance(d time.Duration) { c.at = core.TruncateTimestamp(c.at.Time().Add(d)) }

type mailbox struct {
	mails []VerificationMail
	fail  bool
}

func (m *mailbox) SendVerification(_ context.Context, mail VerificationMail) error {
	if m.fail {
		return errors.New("fixture mail outage")
	}
	m.mails = append(m.mails, mail)
	return nil
}
func email(t *testing.T, text string) webapi.EmailAddress {
	t.Helper()
	v, err := webapi.ParseEmailAddress(text)
	if err != nil {
		t.Fatal(err)
	}
	return v
}

// Cost: a small SQLite database and Argon2's 19 MiB work per verification;
// no network and no wall-clock waiting.
func TestPasswordSessionsAndEmailChange(t *testing.T) {
	ctx := t.Context()
	clock := &clock{core.TruncateTimestamp(time.UnixMilli(1700000000000))}
	c, err := storage.OpenControl(ctx, filepath.Join(t.TempDir(), "control.sqlite"), clock)
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	hasher, err := NewPasswordHasher(ctx)
	if err != nil {
		t.Fatal(err)
	}
	hash, err := hasher.Hash(ctx, webapi.Password("first password"))
	if err != nil {
		t.Fatal(err)
	}
	for _, test := range []struct {
		password string
		known    bool
		want     bool
	}{{"first password", true, true}, {"wrong", true, false}, {"first password", false, false}} {
		var stored *storage.PasswordHash
		if test.known {
			stored = &hash
		}
		ok, err := hasher.Verify(ctx, webapi.Password(test.password), stored)
		if err != nil || ok != test.want {
			t.Fatalf("verify(%v): %v %v", test.known, ok, err)
		}
	}
	user, err := c.CreateMaster(ctx, email(t, "before@example.test"), hash)
	if err != nil {
		t.Fatal(err)
	}
	sessions := WebSessions{c}
	opened, err := sessions.Open(ctx, user.ID)
	if err != nil {
		t.Fatal(err)
	}
	resolved, err := sessions.Resolve(ctx, opened.Token)
	if err != nil || resolved == nil || resolved.User.ID != user.ID {
		t.Fatalf("session: %v %v", resolved, err)
	}
	mail := &mailbox{}
	changes := NewEmailChanges(c, hasher, mail, []byte("fixture code key"))
	start, err := changes.Start(ctx, user.ID, email(t, "after@example.test"), webapi.Password("first password"))
	if err != nil || start.Challenge == nil || len(mail.mails) != 1 {
		t.Fatalf("start: %+v %v", start, err)
	}
	for range 5 {
		outcome, err := changes.Confirm(ctx, user.ID, start.Challenge.ID, "not the code")
		if err != nil || outcome.Kind != storage.ChallengeInvalidCode {
			t.Fatalf("wrong code: %+v %v", outcome, err)
		}
	}
	outcome, err := changes.Confirm(ctx, user.ID, start.Challenge.ID, mail.mails[0].Code)
	if err != nil || outcome.Kind != storage.ChallengeInvalidCode {
		t.Fatalf("used-up code accepted: %+v %v", outcome, err)
	}
	cooled, err := changes.Start(ctx, user.ID, email(t, "after@example.test"), webapi.Password("first password"))
	if err != nil || cooled.Refusal != CoolingDown {
		t.Fatalf("cooldown: %+v %v", cooled, err)
	}
	clock.advance(time.Minute)
	start, err = changes.Start(ctx, user.ID, email(t, "after@example.test"), webapi.Password("first password"))
	if err != nil {
		t.Fatal(err)
	}
	outcome, err = changes.Confirm(ctx, user.ID, start.Challenge.ID, mail.mails[1].Code)
	if err != nil || outcome.Kind != storage.ChallengeChanged || outcome.User.Email.String() != "after@example.test" {
		t.Fatalf("confirmation: %+v %v", outcome, err)
	}
	replay, err := changes.Confirm(ctx, user.ID, start.Challenge.ID, mail.mails[1].Code)
	if err != nil || replay.Kind != storage.ChallengeInvalidCode {
		t.Fatalf("code replayed: %+v %v", replay, err)
	}
	mail.fail = true
	failed, err := changes.Start(ctx, user.ID, email(t, "next@example.test"), webapi.Password("first password"))
	if err != nil || failed.Refusal != MailFailed {
		t.Fatalf("mail failure: %+v %v", failed, err)
	}
	mail.fail = false
	start, err = changes.Start(ctx, user.ID, email(t, "next@example.test"), webapi.Password("first password"))
	if err != nil || start.Challenge == nil {
		t.Fatalf("undelivered challenge blocked retry: %+v %v", start, err)
	}
	nextHash, err := hasher.Hash(ctx, webapi.Password("next password"))
	if err != nil {
		t.Fatal(err)
	}
	if err = c.SetPassword(ctx, user.ID, nextHash); err != nil {
		t.Fatal(err)
	}
	outcome, err = changes.Confirm(ctx, user.ID, start.Challenge.ID, mail.mails[len(mail.mails)-1].Code)
	if err != nil || outcome.Kind != storage.ChallengeInvalidCode {
		t.Fatalf("password change kept challenge: %+v %v", outcome, err)
	}
	if err = sessions.Close(ctx, opened.Token); err != nil {
		t.Fatal(err)
	}
	resolved, err = sessions.Resolve(ctx, opened.Token)
	if err != nil || resolved != nil {
		t.Fatalf("closed session usable: %v %v", resolved, err)
	}
}

// Cost: an in-memory map; all expiration runs on the injected clock.
func TestLoginLockoutUsesEachAddressesLastFailure(t *testing.T) {
	c := &clock{core.TruncateTimestamp(time.UnixMilli(1700000000000))}
	limiter := NewLoginLimiter(c)
	a, b := email(t, "a@example.test"), email(t, "b@example.test")
	for range 4 {
		limiter.Failed(a)
		c.advance(59 * time.Second)
	}
	if limiter.Locked(a) {
		t.Fatal("locked before fifth failure")
	}
	limiter.Failed(a)
	limiter.Failed(b)
	if !limiter.Locked(a) || limiter.Locked(b) {
		t.Fatal("lock crossed address boundary")
	}
	c.advance(59 * time.Second)
	if !limiter.Locked(a) {
		t.Fatal("lock ended early")
	}
	c.advance(time.Second)
	if limiter.Locked(a) {
		t.Fatal("lock did not expire")
	}
	for range 4 {
		limiter.Failed(a)
	}
	limiter.Succeeded(a)
	limiter.Failed(a)
	if limiter.Locked(a) {
		t.Fatal("success did not reset failures")
	}
}
