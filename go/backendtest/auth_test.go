package backendtest_test

import (
	"net/http"
	"slices"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/go/backendtest"
)

// cookieAttributes is a Set-Cookie value's parts, lowercased: the name and value
// first, then each attribute.
func cookieAttributes(cookie string) []string {
	parts := strings.Split(cookie, ";")
	for i := range parts {
		parts[i] = strings.ToLower(strings.TrimSpace(parts[i]))
	}
	return parts
}

// sessionCookie is the one session cookie the answer sets, or fails the test.
func sessionCookie(t *testing.T, answer *backendtest.Answer) string {
	t.Helper()
	cookies := answer.SessionCookies()
	if len(cookies) == 0 {
		t.Fatalf("the answer sets no session cookie: %d %s", answer.Status, answer.Body)
	}
	return cookies[0]
}

// mailCode is the code of the index'th verification mail the backend sent, or
// fails the test when it sent fewer.
func mailCode(t *testing.T, b *backendtest.Backend, index int) string {
	t.Helper()
	mail := b.Control.Mail()
	if len(mail) <= index {
		t.Fatalf("the backend sent %d verification mails, not %d", len(mail), index+1)
	}
	return mail[index].Code
}

// Cost: one backend, about a second: the start and a password hash per login.
func TestSetupCreatesTheMasterOnceAndSignsItIn(t *testing.T) {
	t.Parallel()
	b := backendtest.New(t).Start()
	if !b.Get("/api/setup", nil).Expect(http.StatusOK).At("needed").(bool) {
		t.Fatal("a fresh instance needs setup")
	}

	answer := b.Post("/api/setup", nil, backendtest.Map{"email": "  Master@Example.TEST ", "password": backendtest.MasterPassword})
	answer.Expect(http.StatusCreated)
	master := b.SessionFromAnswer(answer)
	if master.User.Email != backendtest.MasterEmail || master.User.Role != "master" || master.User.Nickname != "" {
		t.Fatalf("the master is %+v", master.User)
	}
	token, _ := strings.CutPrefix(master.Cookie, backendtest.SessionCookie+"=")
	if len(token) < 40 || strings.Trim(token, "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789") != "" {
		t.Fatalf("the token is %q", token)
	}
	// The cookie lives as long as the session: 30 days from the setup, on the
	// backend's clock, which stands still at its start.
	attributes := cookieAttributes(sessionCookie(t, answer))
	for _, attribute := range []string{"httponly", "samesite=lax", "path=/", "expires=sat, 24 oct 2026 08:00:00 gmt"} {
		if !slices.Contains(attributes, attribute) {
			t.Fatalf("%q is not among %v", attribute, attributes)
		}
	}
	if slices.Contains(attributes, "secure") {
		t.Fatalf("a request over HTTP gets a cookie that is secure: %v", attributes)
	}

	if b.Get("/api/setup", nil).Expect(http.StatusOK).At("needed").(bool) {
		t.Fatal("a set up instance needs no setup")
	}
	again := b.Post("/api/setup", nil, backendtest.Map{"email": "other@example.test", "password": "other-pass-1"})
	if status, code := again.Refusal(); status != http.StatusNotFound || code != "already_set_up" {
		t.Fatalf("a second setup is %d %s", status, code)
	}
	if len(again.SessionCookies()) != 0 {
		t.Fatalf("a refused setup sets a cookie: %v", again.SessionCookies())
	}

	me := b.Get("/api/auth/me", master)
	me.Expect(http.StatusOK)
	if me.Str("user.id") != master.User.ID || me.Str("user.email") != backendtest.MasterEmail {
		t.Fatalf("the caller is %s", me.Body)
	}
	b.Stop()
}

// Cost: one backend, about a second.
func TestConcurrentSetupsCreateOneMaster(t *testing.T) {
	t.Parallel()
	b := backendtest.New(t).Start()
	statuses := make([]int, 2)
	emails := []string{"first@example.test", "second@example.test"}
	backendtest.Concurrently(len(emails), func(index int) {
		answer, err := b.TryDo(backendtest.Request{
			Method: http.MethodPost, Path: "/api/setup",
			Body: backendtest.Map{"email": emails[index], "password": backendtest.MasterPassword},
		})
		if err != nil {
			t.Error(err)
			return
		}
		statuses[index] = answer.Status
	})
	slices.Sort(statuses)
	if !slices.Equal(statuses, []int{http.StatusCreated, http.StatusNotFound}) {
		t.Fatalf("two setups at once answer %v", statuses)
	}
	b.Stop()
}

// wantRefusal fails the test unless the answer is the refusal.
func wantRefusal(t *testing.T, answer *backendtest.Answer, status int, code string, what string) {
	t.Helper()
	if gotStatus, gotCode := answer.Refusal(); gotStatus != status || gotCode != code {
		t.Fatalf("%s is %d %s, not %d %s: %s", what, gotStatus, gotCode, status, code, answer.Body)
	}
}

// Cost: one backend, about a second.
func TestEveryOtherAPIPathWantsALiveSession(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()

	for _, route := range []struct{ method, path string }{
		{http.MethodGet, "/api/auth/me"},
		{http.MethodGet, "/api/no-such-resource"},
		{http.MethodGet, "/api"},
		{http.MethodGet, "/api/setup/more"},
		{http.MethodDelete, "/api/auth/me"},
	} {
		anonymous := b.Do(backendtest.Request{Method: route.method, Path: route.path})
		wantRefusal(t, anonymous, http.StatusUnauthorized, "unauthenticated", route.method+" "+route.path)
		if len(anonymous.SessionCookies()) != 0 {
			t.Fatalf("%s %s sets a cookie", route.method, route.path)
		}
	}

	missing := b.Get("/api/no-such-resource", master)
	wantRefusal(t, missing, http.StatusNotFound, "not_found", "a missing resource")
	if missing.ErrorMessage() != "No route for GET /api/no-such-resource" {
		t.Fatalf("the message is %q", missing.ErrorMessage())
	}
	wrongMethod := b.Delete("/api/auth/me", master)
	wantRefusal(t, wrongMethod, http.StatusNotFound, "not_found", "a wrong method")

	// A cookie that names no session is refused and cleared.
	forged := b.Get("/api/auth/me", &backendtest.Session{Cookie: backendtest.SessionCookie + "=not-a-session"})
	wantRefusal(t, forged, http.StatusUnauthorized, "unauthenticated", "a forged cookie")
	cleared := cookieAttributes(sessionCookie(t, forged))
	if cleared[0] != backendtest.SessionCookie+"=" || !slices.Contains(cleared, "max-age=0") {
		t.Fatalf("the cookie is not cleared: %v", cleared)
	}
	b.Stop()
}

// postFrom is a POST from a page at origin, or from a program that sends no
// origin, with its body typed text/plain, as a page of another site may send it
// without asking the backend first.
func postFrom(b *backendtest.Backend, origin, path string, session *backendtest.Session, body any) *backendtest.Answer {
	request := backendtest.Request{Method: http.MethodPost, Path: path, Session: session, Headers: map[string]string{}}
	if origin != "" {
		request.Headers["Origin"] = origin
	}
	if body != nil {
		request.Raw = []byte(mustJSON(body))
		request.ContentType = "text/plain"
	}
	return b.Do(request)
}

func mustJSON(value any) string {
	return string(backendtest.Marshal(value))
}

// The browser sends the session cookie from every page of the product's site, an
// expose's among them, so a page of another site or an expose may neither set
// the instance up, nor sign the browser in, nor act with its session. The
// product's page may, and so may a program that sends no origin, such as curl
// calling the setup API.
//
// Cost: one backend, about a second.
func TestARequestThatCouldActComesFromAPageOfTheProductOrFromNoPage(t *testing.T) {
	t.Parallel()
	b := backendtest.New(t, backendtest.WithExposeDomain("expose.localhost")).Start()
	product := b.URL
	// An expose's origin as the backend prints its URLs: the public URL's scheme
	// and port under the expose domain.
	expose := "http://a1b2c3d4e5.expose.localhost:" + strconv.Itoa(b.Port)
	others := []string{"https://elsewhere.example", expose, "null"}
	credentials := backendtest.Map{"email": backendtest.MasterEmail, "password": backendtest.MasterPassword}

	for _, origin := range others {
		setup := postFrom(b, origin, "/api/setup", nil, credentials)
		wantRefusal(t, setup, http.StatusForbidden, "forbidden_origin", "a setup from "+origin)
		if len(setup.SessionCookies()) != 0 {
			t.Fatalf("a refused setup from %s sets a cookie", origin)
		}
	}
	if !b.Get("/api/setup", nil).At("needed").(bool) {
		t.Fatal("a refused setup set the instance up")
	}
	// The setup API as curl calls it.
	setup := postFrom(b, "", "/api/setup", nil, credentials).Expect(http.StatusCreated)
	master := b.SessionFromAnswer(setup)

	login := postFrom(b, expose, "/api/auth/login", nil, credentials)
	wantRefusal(t, login, http.StatusForbidden, "forbidden_origin", "a login from an expose")
	if len(login.SessionCookies()) != 0 {
		t.Fatal("a refused login sets a cookie")
	}
	postFrom(b, product, "/api/auth/login", nil, credentials).Expect(http.StatusOK)

	// An administrator's request from another page creates no account, and a
	// sign-out from another page ends no session.
	account := func(email string) backendtest.Map {
		return backendtest.Map{"email": email, "password": "user-pass-1", "role": "user"}
	}
	for _, origin := range others {
		created := postFrom(b, origin, "/api/users", master, account("forged@example.test"))
		wantRefusal(t, created, http.StatusForbidden, "forbidden_origin", "an account from "+origin)
		out := postFrom(b, origin, "/api/auth/logout", master, nil)
		wantRefusal(t, out, http.StatusForbidden, "forbidden_origin", "a sign-out from "+origin)
	}
	users := b.Get("/api/users", master).At("users").([]any)
	if len(users) != 1 || backendtest.At(users, "0.email") != backendtest.MasterEmail {
		t.Fatalf("the accounts are %v", users)
	}
	for origin, email := range map[string]string{product: "page@example.test", "": "curl@example.test"} {
		postFrom(b, origin, "/api/users", master, account(email)).Expect(http.StatusCreated)
	}
	postFrom(b, product, "/api/auth/logout", master, nil).Expect(http.StatusNoContent)
	b.Stop()
}

// A browser sends Fetch Metadata with every request Origin must come with, so a
// request with Sec-Fetch-Site and without Origin lost its Origin at a proxy in
// front of the backend, which turns the check off. The request passes, as any
// request without Origin does, and the backend's log says once that the proxy
// drops Origin; curl's request, which carries neither, says nothing.
//
// The log is read from the backend's output: its wording ("drops the Origin
// header") is the Rust backend's, which the Go backend keeps or the scenario
// changes.
//
// Cost: one backend, about a second.
func TestARequestWhoseOriginAProxyDroppedPassesAndTheLogSaysSoOnce(t *testing.T) {
	t.Parallel()
	b := backendtest.New(t).Start()
	login := func(fetchSite string) *backendtest.Answer {
		request := backendtest.Request{Method: http.MethodPost, Path: "/api/auth/login", Raw: []byte("{}")}
		if fetchSite != "" {
			request.Headers = map[string]string{"Sec-Fetch-Site": fetchSite}
		}
		return b.Do(request)
	}
	warnings := func() int {
		return strings.Count(b.Logs(), "drops the Origin header")
	}

	wantRefusal(t, login(""), http.StatusBadRequest, "invalid_body", "a request without Origin")
	if warnings() != 0 {
		t.Fatalf("a request that carries neither header is logged: %s", b.Logs())
	}
	for range 2 {
		wantRefusal(t, login("same-origin"), http.StatusBadRequest, "invalid_body", "a request with Fetch Metadata")
	}
	if warnings() != 1 {
		t.Fatalf("the log says %d times that the proxy drops Origin: %s", warnings(), b.Logs())
	}
	b.Stop()
}

// Cost: one backend, about a second.
func TestARequestOverHTTPSGetsASecureCookie(t *testing.T) {
	t.Parallel()
	b, _ := backendtest.New(t).StartSetUp()
	forwarded := b.Do(backendtest.Request{
		Method: http.MethodPost, Path: "/api/auth/login",
		Body:    backendtest.Map{"email": backendtest.MasterEmail, "password": backendtest.MasterPassword},
		Headers: map[string]string{"X-Forwarded-Proto": "https, http"},
	}).Expect(http.StatusOK)
	cookie := forwarded.Header.Get("Set-Cookie")
	if !slices.Contains(cookieAttributes(cookie), "secure") {
		t.Fatalf("the cookie is not secure: %s", cookie)
	}
	b.Stop()
}

// Cost: one backend, about a second, and seven password checks.
func TestLoginLocksOutAfterFiveFailuresAndLogoutEndsTheSession(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()

	wrong := b.LoginAnswer(backendtest.MasterEmail, "nope-nope")
	wantRefusal(t, wrong, http.StatusUnauthorized, "invalid_credentials", "a wrong password")
	if len(wrong.SessionCookies()) != 0 {
		t.Fatal("a failed login sets a cookie")
	}

	signedIn := b.Login(backendtest.MasterEmail, backendtest.MasterPassword)
	if signedIn.User.ID != master.User.ID || signedIn.Cookie == master.Cookie {
		t.Fatalf("a second login is the same session: %v", signedIn)
	}

	// The success cleared the count: five fresh failures lock the address, and
	// then even the right password is refused.
	for range 5 {
		b.LoginAnswer(backendtest.MasterEmail, "nope-nope").Expect(http.StatusUnauthorized)
	}
	locked := b.LoginAnswer(backendtest.MasterEmail, backendtest.MasterPassword)
	wantRefusal(t, locked, http.StatusTooManyRequests, "too_many_attempts", "a login after five failures")
	// The lock never touches sessions already open.
	b.Get("/api/auth/me", signedIn).Expect(http.StatusOK)

	out := b.Post("/api/auth/logout", signedIn, backendtest.Map{}).Expect(http.StatusNoContent)
	if !slices.Contains(cookieAttributes(sessionCookie(t, out)), "max-age=0") {
		t.Fatalf("the sign-out does not clear the cookie: %v", out.SessionCookies())
	}
	b.Get("/api/auth/me", signedIn).Expect(http.StatusUnauthorized)
	b.Get("/api/auth/me", master).Expect(http.StatusOK)
	b.Stop()
}

// Cost: one backend, about a second, and seven password checks.
func TestAnAddressWithoutAnAccountIsLockedOutToo(t *testing.T) {
	t.Parallel()
	b, _ := backendtest.New(t).StartSetUp()
	for range 5 {
		failed := b.LoginAnswer("nobody@example.test", "nope-nope")
		wantRefusal(t, failed, http.StatusUnauthorized, "invalid_credentials", "a login without an account")
	}
	locked := b.LoginAnswer("NOBODY@example.test", "nope-nope")
	wantRefusal(t, locked, http.StatusTooManyRequests, "too_many_attempts", "a sixth login")
	b.Login(backendtest.MasterEmail, backendtest.MasterPassword)
	b.Stop()
}

// Cost: one backend, about a second.
func TestAUserChangesTheirOwnPasswordWithTheCurrentOne(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()
	change := func(current, next string) *backendtest.Answer {
		return b.Put("/api/auth/password", master, backendtest.Map{"current": current, "next": next})
	}

	wantRefusal(t, change("wrong-wrong", "second-pass-2"), http.StatusUnauthorized, "invalid_credentials", "a wrong current password")
	short := change(backendtest.MasterPassword, "short")
	wantRefusal(t, short, http.StatusBadRequest, "invalid_body", "a short password")
	if !strings.Contains(short.ErrorMessage(), "next") {
		t.Fatalf("the message %q does not name the field", short.ErrorMessage())
	}
	change(backendtest.MasterPassword, "second-pass-2").Expect(http.StatusNoContent)

	b.LoginAnswer(backendtest.MasterEmail, backendtest.MasterPassword).Expect(http.StatusUnauthorized)
	if b.Login(backendtest.MasterEmail, "second-pass-2").User.ID != master.User.ID {
		t.Fatal("the new password signs in another account")
	}
	b.Stop()
}

// Cost: one backend, about a second.
func TestEmailIdentityIsNormalizedAndTheNicknamePersists(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()

	signedIn := b.Login("  "+strings.ToUpper(backendtest.MasterEmail)+"  ", backendtest.MasterPassword)
	if signedIn.User.Email != backendtest.MasterEmail {
		t.Fatalf("the address is %q", signedIn.User.Email)
	}
	invalid := b.LoginAnswer("invalid", backendtest.MasterPassword)
	wantRefusal(t, invalid, http.StatusBadRequest, "invalid_body", "an address that is none")
	if !strings.Contains(invalid.ErrorMessage(), "email") {
		t.Fatalf("the message %q does not name the field", invalid.ErrorMessage())
	}

	renamed := b.Patch("/api/auth/me", signedIn, backendtest.Map{"nickname": "  New name  "})
	if renamed.Str("user.email") != backendtest.MasterEmail || renamed.Str("user.nickname") != "New name" {
		t.Fatalf("the renamed account is %s", renamed.Body)
	}
	if nickname := b.Get("/api/auth/me", master).Str("user.nickname"); nickname != "New name" {
		t.Fatalf("another session sees the nickname %q", nickname)
	}
	for _, refused := range []backendtest.Map{{"nickname": " "}, {"nickname": "Ana", "role": "admin"}} {
		wantRefusal(t, b.Patch("/api/auth/me", signedIn, refused), http.StatusBadRequest, "invalid_body", "the patch")
	}

	unavailable := b.Post("/api/auth/email", signedIn, backendtest.Map{"email": "next@example.test", "password": backendtest.MasterPassword})
	wantRefusal(t, unavailable, http.StatusServiceUnavailable, "mail_unavailable", "an email change without a mail sender")
	b.Stop()
}

const day = 24 * time.Hour

// Cost: one backend, about a second.
func TestASessionSlidesWhileUsedAndExpiresWhenSilent(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()

	b.Control.AdvanceClock(10 * day)
	early := b.Get("/api/auth/me", master).Expect(http.StatusOK)
	if cookies := early.SessionCookies(); len(cookies) != 0 {
		t.Fatalf("a session with more than half its life left is renewed: %v", cookies)
	}

	// 16 days in, less than 15 remain: the session renews to 30 days from now.
	b.Control.AdvanceClock(6 * day)
	renewed := b.Get("/api/auth/me", master).Expect(http.StatusOK)
	cookie := cookieAttributes(sessionCookie(t, renewed))
	if cookie[0] != strings.ToLower(master.Cookie) || !slices.Contains(cookie, "expires=mon, 09 nov 2026 08:00:00 gmt") {
		t.Fatalf("the renewed cookie is %v", cookie)
	}

	// Past the first expiry, inside the renewed one.
	b.Control.AdvanceClock(20 * day)
	b.Get("/api/auth/me", master).Expect(http.StatusOK)

	b.Control.AdvanceClock(31 * day)
	expired := b.Get("/api/auth/me", master)
	wantRefusal(t, expired, http.StatusUnauthorized, "unauthenticated", "an expired session")
	if !slices.Contains(cookieAttributes(sessionCookie(t, expired)), "max-age=0") {
		t.Fatalf("an expired session's cookie is not cleared: %v", expired.SessionCookies())
	}
	b.Stop()
}

// startEmailChange asks for the account's email to change to email.
func startEmailChange(b *backendtest.Backend, session *backendtest.Session, email string) *backendtest.Answer {
	return b.Post("/api/auth/email", session, backendtest.Map{"email": email, "password": backendtest.MasterPassword})
}

// Cost: one backend started twice, about two seconds.
func TestAnEmailChangeNeedsADeliveredUnexpiredSingleUseCodeAndSurvivesRestart(t *testing.T) {
	t.Parallel()
	h := backendtest.New(t, backendtest.WithMail())
	b, master := h.StartSetUp()

	wantRefusal(t, b.Post("/api/auth/email", master, backendtest.Map{"email": "next@example.test", "password": "wrong-password"}),
		http.StatusUnauthorized, "invalid_credentials", "a wrong password")
	wantRefusal(t, startEmailChange(b, master, backendtest.MasterEmail), http.StatusConflict, "email_taken", "the account's own address")

	started := startEmailChange(b, master, "NEXT@example.test").Expect(http.StatusAccepted)
	mail := b.Control.Mail()
	if len(mail) != 1 || mail[0].Email != "next@example.test" {
		t.Fatalf("the mail sent is %+v", mail)
	}
	challengeID := started.Str("challenge.id")
	if started.Str("challenge.email") != mail[0].Email || started.Str("challenge.expiresAt") != "2026-09-24T08:10:00.000Z" {
		t.Fatalf("the challenge is %s", started.Body)
	}
	if strings.Contains(started.Text(), mail[0].Code) {
		t.Fatal("the answer carries the code")
	}
	wantRefusal(t, startEmailChange(b, master, "next@example.test"), http.StatusTooManyRequests, "too_many_attempts", "a second code within the cooldown")
	for _, malformed := range []string{"abcdef", "١٢٣٤٥٦"} {
		wantRefusal(t, b.Post("/api/auth/email/confirm", master, backendtest.Map{"id": challengeID, "code": malformed}),
			http.StatusBadRequest, "invalid_body", "the code "+malformed)
	}

	// The challenge is a record, which a restart keeps. The clock is the new
	// process's, which starts where the first did.
	b.Stop()
	b = h.Start()
	master = b.Login(backendtest.MasterEmail, backendtest.MasterPassword)
	confirm := backendtest.Map{"id": challengeID, "code": mail[0].Code}
	b.Post("/api/auth/email/confirm", master, confirm).Expect(http.StatusOK)
	if email := b.Get("/api/auth/me", master).Str("user.email"); email != "next@example.test" {
		t.Fatalf("the account's address is %s", email)
	}
	wantRefusal(t, b.Post("/api/auth/email/confirm", master, confirm), http.StatusBadRequest, "invalid_code", "a code used twice")
	wantRefusal(t, b.LoginAnswer(backendtest.MasterEmail, backendtest.MasterPassword), http.StatusUnauthorized, "invalid_credentials", "the old address")
	if signedIn := b.Login("next@example.test", backendtest.MasterPassword); signedIn.User.ID != master.User.ID {
		t.Fatalf("the new address signs in as %s", signedIn.User.ID)
	}

	// After the cooldown a new code goes out; ten minutes later it is dead.
	b.Control.AdvanceClock(time.Minute)
	later := startEmailChange(b, master, "later@example.test").Expect(http.StatusAccepted)
	b.Control.AdvanceClock(10 * time.Minute)
	mail = b.Control.Mail()
	if len(mail) != 1 {
		t.Fatalf("the new process's mailbox holds %+v", mail)
	}
	wantRefusal(t, b.Post("/api/auth/email/confirm", master, backendtest.Map{"id": later.Str("challenge.id"), "code": mail[0].Code}),
		http.StatusBadRequest, "invalid_code", "a code after its ten minutes")
	b.Stop()
}

// Cost: one backend, about a second.
func TestAFailedDeliveryAllowsARetryAndWrongCodesOrANewPasswordEndAChallenge(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t, backendtest.WithMail()).StartSetUp()
	start := func() *backendtest.Answer { return startEmailChange(b, master, "new@example.test") }

	b.Control.FailMail(true)
	wantRefusal(t, start(), http.StatusServiceUnavailable, "mail_failed", "a failed delivery")
	// The failed code holds back no retry.
	b.Control.FailMail(false)
	challenge := start().Expect(http.StatusAccepted).Str("challenge.id")
	code := mailCode(t, b, 0)
	wrong := "000000"
	if code == wrong {
		wrong = "111111"
	}
	for range 5 {
		wantRefusal(t, b.Post("/api/auth/email/confirm", master, backendtest.Map{"id": challenge, "code": wrong}),
			http.StatusBadRequest, "invalid_code", "a wrong code")
	}
	wantRefusal(t, b.Post("/api/auth/email/confirm", master, backendtest.Map{"id": challenge, "code": code}),
		http.StatusBadRequest, "invalid_code", "the right code after five wrong ones")

	b.Control.AdvanceClock(time.Minute)
	challenge = start().Expect(http.StatusAccepted).Str("challenge.id")
	code = mailCode(t, b, 1)
	b.Put("/api/auth/password", master, backendtest.Map{"current": backendtest.MasterPassword, "next": "new-password-2"}).Expect(http.StatusNoContent)
	wantRefusal(t, b.Post("/api/auth/email/confirm", master, backendtest.Map{"id": challenge, "code": code}),
		http.StatusBadRequest, "invalid_code", "a code after a new password")
	b.Stop()
}
