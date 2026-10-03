package backend_test

import (
	"context"
	"log/slog"
	"net/http"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/google/go-cmp/cmp"
	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/webapi"
)

// accountHarness starts the account scenarios with the shared isolated backend fixture.
func accountHarness(ctx context.Context, t *testing.T) *backendtest.Harness {
	t.Helper()
	manager, err := backendtest.StartScriptedManager(context.WithoutCancel(ctx), t)
	if err != nil {
		t.Fatal(err)
	}
	h, err := backendtest.NewHarness(ctx, t, manager.Socket())
	if err != nil {
		t.Fatal(err)
	}
	return h
}

// accountRequest sends the account scenario's deliberately valid or invalid wire body.
func accountRequest(ctx context.Context, t *testing.T, b *backendtest.TestBackend, method, path string, s *backendtest.Session, body string) backendtest.Answer {
	t.Helper()
	var cookie *string
	if s != nil {
		cookie = &s.Cookie
	}
	var data []byte
	if body != "" {
		data = []byte(body)
	}
	a, err := b.Send(ctx, method, path, cookie, data)
	if err != nil {
		t.Fatal(err)
	}
	return a
}

// accountDecode validates an account response at its generated contract boundary.
func accountDecode[T any](t *testing.T, a backendtest.Answer, decode func([]byte) (T, error)) T {
	t.Helper()
	value, err := decode(a.Body)
	if err != nil {
		t.Fatalf("HTTP %d: %s: %v", a.Status, a.Body, err)
	}
	return value
}

// accountEqual reports the observable account contract difference.
func accountEqual[T any](t *testing.T, got, want T) {
	t.Helper()
	if diff := cmp.Diff(want, got); diff != "" {
		t.Fatalf("(-want +got):\n%s", diff)
	}
}

// accountRefusal checks the status and public account error together.
func accountRefusal(t *testing.T, a backendtest.Answer, status int, code webapi.ErrorCode) webapi.ErrorBody {
	t.Helper()
	accountEqual(t, a.Status, status)
	e := accountDecode(t, a, webapi.DecodeErrorBody)
	accountEqual(t, e.Code, code)
	return e
}

// accountStart creates the fixture master through the public setup route.
func accountStart(ctx context.Context, t *testing.T, h *backendtest.Harness) (*backendtest.TestBackend, backendtest.Session) {
	t.Helper()
	b, s, err := h.StartSetUp(ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	return b, s
}

// accountLogin signs in through the public account route.
func accountLogin(ctx context.Context, t *testing.T, b *backendtest.TestBackend, email, password string) backendtest.Session {
	t.Helper()
	s, err := b.Login(ctx, email, password)
	if err != nil {
		t.Fatal(err)
	}
	return s
}

// accountJSON preserves fixture object order and serde-compatible escaping.
func accountJSON(t *testing.T, fields ...contract.Field) string {
	t.Helper()
	data, err := contract.EncodeObject(fields)
	if err != nil {
		t.Fatal(err)
	}
	return string(data)
}

// accountCredentials makes login and setup bodies, including invalid email spellings.
func accountCredentials(t *testing.T, email, password string) string {
	return accountJSON(t, contract.Field{Name: "email", Value: email}, contract.Field{Name: "password", Value: password})
}

// accountCookie checks attributes case insensitively as browsers interpret them.
func accountCookie(t *testing.T, a backendtest.Answer, attribute string) {
	t.Helper()
	cookies := a.SessionCookies()
	if len(cookies) != 1 {
		t.Fatalf("session cookies: %v", cookies)
	}
	parts := strings.Split(strings.ToLower(cookies[0]), ";")
	for i := range parts {
		parts[i] = strings.TrimSpace(parts[i])
	}
	if !slices.Contains(parts, strings.ToLower(attribute)) {
		t.Fatalf("cookie %q lacks %q", cookies[0], attribute)
	}
}

func TestSetupCreatesTheMasterOnceAndSignsItIn(t *testing.T) {
	ctx := t.Context()
	h := accountHarness(ctx, t)
	b, err := h.Start(ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	accountEqual(t, accountDecode(t, accountRequest(ctx, t, b, "GET", "/api/setup", nil, ""), webapi.DecodeSetupStatus).Needed, true)
	a := accountRequest(ctx, t, b, "POST", "/api/setup", nil, accountCredentials(t, "  Master@Example.TEST ", backendtest.MasterPassword))
	accountEqual(t, a.Status, 201)
	master, err := backendtest.SessionFrom(a)
	if err != nil {
		t.Fatal(err)
	}
	accountEqual(t, string(master.User.Email), backendtest.MasterEmail)
	accountEqual(t, master.User.Role, webapi.RoleMaster)
	accountEqual(t, master.User.Nickname, "")
	token := strings.TrimPrefix(master.Cookie, backendtest.SessionCookie+"=")
	if len(token) < 40 || strings.ContainsFunc(token, func(r rune) bool { return (r < 'a' || r > 'z') && (r < 'A' || r > 'Z') && (r < '0' || r > '9') }) {
		t.Fatalf("token: %q", token)
	}
	for _, attr := range []string{"httponly", "samesite=lax", "path=/", "expires=sat, 24 oct 2026 08:00:00 gmt"} {
		accountCookie(t, a, attr)
	}
	if strings.Contains(strings.ToLower(a.SessionCookies()[0]), "; secure") {
		t.Fatal("HTTP cookie is secure")
	}
	accountEqual(t, accountDecode(t, accountRequest(ctx, t, b, "GET", "/api/setup", nil, ""), webapi.DecodeSetupStatus).Needed, false)
	again := accountRequest(ctx, t, b, "POST", "/api/setup", nil, accountCredentials(t, "other@example.test", "other-pass-1"))
	accountRefusal(t, again, 404, webapi.ErrorCodeAlreadySetUp)
	accountEqual(t, len(again.SessionCookies()), 0)
	accountEqual(t, accountDecode(t, accountRequest(ctx, t, b, "GET", "/api/auth/me", &master, ""), webapi.DecodeIdentity).User, master.User)
}

func TestConcurrentSetupsCreateOneMaster(t *testing.T) {
	ctx := t.Context()
	h := accountHarness(ctx, t)
	b, err := h.Start(ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	bodies := []string{accountCredentials(t, "first@example.test", backendtest.MasterPassword), accountCredentials(t, "second@example.test", backendtest.MasterPassword)}
	answers := make([]backendtest.Answer, 2)
	errs := make([]error, 2)
	var wg sync.WaitGroup
	for i := range bodies {
		wg.Go(func() { answers[i], errs[i] = b.Post(ctx, "/api/setup", nil, []byte(bodies[i])) })
	}
	wg.Wait()
	for _, err := range errs {
		if err != nil {
			t.Fatal(err)
		}
	}
	statuses := []int{answers[0].Status, answers[1].Status}
	slices.Sort(statuses)
	accountEqual(t, statuses, []int{201, 404})
}

func TestEveryOtherAPIPathWantsALiveSession(t *testing.T) {
	ctx := t.Context()
	b, master := accountStart(ctx, t, accountHarness(ctx, t))
	for _, route := range [][2]string{{"GET", "/api/auth/me"}, {"GET", "/api/no-such-resource"}, {"GET", "/api"}, {"GET", "/api/setup/more"}, {"DELETE", "/api/auth/me"}} {
		a := accountRequest(ctx, t, b, route[0], route[1], nil, "")
		accountRefusal(t, a, 401, webapi.ErrorCodeUnauthenticated)
		accountEqual(t, len(a.SessionCookies()), 0)
	}
	e := accountRefusal(t, accountRequest(ctx, t, b, "GET", "/api/no-such-resource", &master, ""), 404, webapi.ErrorCodeNotFound)
	accountEqual(t, e.Message, "No route for GET /api/no-such-resource")
	accountRefusal(t, accountRequest(ctx, t, b, "DELETE", "/api/auth/me", &master, ""), 404, webapi.ErrorCodeNotFound)
	forged := backendtest.Session{Cookie: backendtest.SessionCookie + "=not-a-session"}
	a := accountRequest(ctx, t, b, "GET", "/api/auth/me", &forged, "")
	accountRefusal(t, a, 401, webapi.ErrorCodeUnauthenticated)
	accountCookie(t, a, backendtest.SessionCookie+"=")
	accountCookie(t, a, "max-age=0")
}

func TestARequestOverHTTPSGetsASecureCookie(t *testing.T) {
	ctx := t.Context()
	b, _ := accountStart(ctx, t, accountHarness(ctx, t))
	r, err := b.Response(ctx, "POST", "/api/auth/login", nil, http.Header{"X-Forwarded-Proto": []string{"https, http"}}, strings.NewReader(accountCredentials(t, backendtest.MasterEmail, backendtest.MasterPassword)))
	if err != nil {
		t.Fatal(err)
	}
	a, err := backendtest.ReadAnswer(ctx, r)
	if err != nil {
		t.Fatal(err)
	}
	accountEqual(t, a.Status, 200)
	accountCookie(t, a, "secure")
}

func TestLoginLocksOutAfterFiveFailuresAndLogoutEndsTheSession(t *testing.T) {
	ctx := t.Context()
	b, master := accountStart(ctx, t, accountHarness(ctx, t))
	login := func(password string) backendtest.Answer {
		return accountRequest(ctx, t, b, "POST", "/api/auth/login", nil, accountCredentials(t, backendtest.MasterEmail, password))
	}
	wrong := login("nope-nope")
	accountRefusal(t, wrong, 401, webapi.ErrorCodeInvalidCredentials)
	accountEqual(t, len(wrong.SessionCookies()), 0)
	signed := accountLogin(ctx, t, b, backendtest.MasterEmail, backendtest.MasterPassword)
	accountEqual(t, signed.User.ID, master.User.ID)
	if signed.Cookie == master.Cookie {
		t.Fatal("login reused cookie")
	}
	for range 5 {
		accountEqual(t, login("nope-nope").Status, 401)
	}
	accountRefusal(t, login(backendtest.MasterPassword), 429, webapi.ErrorCodeTooManyAttempts)
	accountEqual(t, accountRequest(ctx, t, b, "GET", "/api/auth/me", &signed, "").Status, 200)
	out := accountRequest(ctx, t, b, "POST", "/api/auth/logout", &signed, "{}")
	accountEqual(t, out.Status, 204)
	accountCookie(t, out, "max-age=0")
	accountEqual(t, accountRequest(ctx, t, b, "GET", "/api/auth/me", &signed, "").Status, 401)
	accountEqual(t, accountRequest(ctx, t, b, "GET", "/api/auth/me", &master, "").Status, 200)
}

func TestAnAddressWithoutAnAccountIsLockedOutToo(t *testing.T) {
	ctx := t.Context()
	b, _ := accountStart(ctx, t, accountHarness(ctx, t))
	for range 5 {
		accountRefusal(t, accountRequest(ctx, t, b, "POST", "/api/auth/login", nil, accountCredentials(t, "nobody@example.test", "nope-nope")), 401, webapi.ErrorCodeInvalidCredentials)
	}
	accountRefusal(t, accountRequest(ctx, t, b, "POST", "/api/auth/login", nil, accountCredentials(t, "NOBODY@example.test", "nope-nope")), 429, webapi.ErrorCodeTooManyAttempts)
	accountLogin(ctx, t, b, backendtest.MasterEmail, backendtest.MasterPassword)
}

func TestAUserChangesTheirOwnPasswordWithTheCurrentOne(t *testing.T) {
	ctx := t.Context()
	b, master := accountStart(ctx, t, accountHarness(ctx, t))
	change := func(current, next string) backendtest.Answer {
		return accountRequest(ctx, t, b, "PUT", "/api/auth/password", &master, accountJSON(t, contract.Field{Name: "current", Value: current}, contract.Field{Name: "next", Value: next}))
	}
	accountRefusal(t, change("wrong-wrong", "second-pass-2"), 401, webapi.ErrorCodeInvalidCredentials)
	e := accountRefusal(t, change(backendtest.MasterPassword, "short"), 400, webapi.ErrorCodeInvalidBody)
	if !strings.Contains(e.Message, "next") {
		t.Fatal(e.Message)
	}
	accountEqual(t, change(backendtest.MasterPassword, "second-pass-2").Status, 204)
	accountEqual(t, accountRequest(ctx, t, b, "POST", "/api/auth/login", nil, accountCredentials(t, backendtest.MasterEmail, backendtest.MasterPassword)).Status, 401)
	accountEqual(t, accountLogin(ctx, t, b, backendtest.MasterEmail, "second-pass-2").User.ID, master.User.ID)
}

func TestASessionSlidesWhileUsedAndExpiresWhenSilent(t *testing.T) {
	ctx := t.Context()
	h := accountHarness(ctx, t)
	b, master := accountStart(ctx, t, h)
	advance := func(days int) {
		if err := h.Clock.Advance(time.Duration(days) * 24 * time.Hour); err != nil {
			t.Fatal(err)
		}
	}
	advance(10)
	early := accountRequest(ctx, t, b, "GET", "/api/auth/me", &master, "")
	accountEqual(t, early.Status, 200)
	accountEqual(t, len(early.SessionCookies()), 0)
	advance(6)
	renewed := accountRequest(ctx, t, b, "GET", "/api/auth/me", &master, "")
	accountEqual(t, renewed.Status, 200)
	accountCookie(t, renewed, master.Cookie)
	accountCookie(t, renewed, "expires=mon, 09 nov 2026 08:00:00 gmt")
	advance(20)
	accountEqual(t, accountRequest(ctx, t, b, "GET", "/api/auth/me", &master, "").Status, 200)
	advance(31)
	expired := accountRequest(ctx, t, b, "GET", "/api/auth/me", &master, "")
	accountRefusal(t, expired, 401, webapi.ErrorCodeUnauthenticated)
	accountCookie(t, expired, "max-age=0")
}

func TestEmailIdentityIsNormalizedAndTheNicknamePersists(t *testing.T) {
	ctx := t.Context()
	b, master := accountStart(ctx, t, accountHarness(ctx, t))
	signed := accountLogin(ctx, t, b, "  "+strings.ToUpper(backendtest.MasterEmail)+"  ", backendtest.MasterPassword)
	accountEqual(t, string(signed.User.Email), backendtest.MasterEmail)
	e := accountRefusal(t, accountRequest(ctx, t, b, "POST", "/api/auth/login", nil, accountCredentials(t, "invalid", backendtest.MasterPassword)), 400, webapi.ErrorCodeInvalidBody)
	if !strings.Contains(e.Message, "email") {
		t.Fatal(e.Message)
	}
	user := accountDecode(t, accountRequest(ctx, t, b, "PATCH", "/api/auth/me", &signed, `{"nickname":"  New name  "}`), webapi.DecodeIdentity).User
	accountEqual(t, string(user.Email), backendtest.MasterEmail)
	accountEqual(t, user.Nickname, "New name")
	accountEqual(t, accountDecode(t, accountRequest(ctx, t, b, "GET", "/api/auth/me", &master, ""), webapi.DecodeIdentity).User.Nickname, "New name")
	for _, body := range []string{`{"nickname":" "}`, `{"nickname":"Ana","role":"admin"}`} {
		accountRefusal(t, accountRequest(ctx, t, b, "PATCH", "/api/auth/me", &signed, body), 400, webapi.ErrorCodeInvalidBody)
	}
	accountRefusal(t, accountRequest(ctx, t, b, "POST", "/api/auth/email", &signed, accountCredentials(t, "next@example.test", backendtest.MasterPassword)), 503, webapi.ErrorCodeMailUnavailable)
}

func TestAnEmailChangeNeedsADeliveredUnexpiredSingleUseCodeAndSurvivesRestart(t *testing.T) {
	ctx := t.Context()
	h := accountHarness(ctx, t)
	h.Config.AccountMail = h.Mailbox
	b, master := accountStart(ctx, t, h)
	start := func(email string) backendtest.Answer {
		return accountRequest(ctx, t, b, "POST", "/api/auth/email", &master, accountCredentials(t, email, backendtest.MasterPassword))
	}
	confirm := func(id string, code string) backendtest.Answer {
		return accountRequest(ctx, t, b, "POST", "/api/auth/email/confirm", &master, accountJSON(t, contract.Field{Name: "id", Value: id}, contract.Field{Name: "code", Value: code}))
	}
	accountRefusal(t, accountRequest(ctx, t, b, "POST", "/api/auth/email", &master, accountCredentials(t, "next@example.test", "wrong-password")), 401, webapi.ErrorCodeInvalidCredentials)
	accountRefusal(t, start(backendtest.MasterEmail), 409, webapi.ErrorCodeEmailTaken)
	started := start("NEXT@example.test")
	accountEqual(t, started.Status, 202)
	challenge := accountDecode(t, started, webapi.DecodeEmailChangeStarted).Challenge
	mail := h.Mailbox.Sent()[0]
	accountEqual(t, string(mail.Email), "next@example.test")
	accountEqual(t, challenge.Email, mail.Email)
	accountEqual(t, string(challenge.ExpiresAt), "2026-09-24T08:10:00.000Z")
	if strings.Contains(string(started.Body), mail.Code) {
		t.Fatal("answer contains code")
	}
	accountRefusal(t, start("next@example.test"), 429, webapi.ErrorCodeTooManyAttempts)
	for _, code := range []string{"abcdef", "١٢٣٤٥٦"} {
		accountRefusal(t, confirm(challenge.ID, code), 400, webapi.ErrorCodeInvalidBody)
	}
	if err := b.Close(ctx); err != nil {
		t.Fatal(err)
	}
	var err error
	b, err = h.Start(ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	master = accountLogin(ctx, t, b, backendtest.MasterEmail, backendtest.MasterPassword)
	confirmed := confirm(challenge.ID, mail.Code)
	accountEqual(t, confirmed.Status, 200)
	accountEqual(t, string(accountDecode(t, confirmed, webapi.DecodeIdentity).User.Email), "next@example.test")
	accountRefusal(t, confirm(challenge.ID, mail.Code), 400, webapi.ErrorCodeInvalidCode)
	accountEqual(t, accountRequest(ctx, t, b, "POST", "/api/auth/login", nil, accountCredentials(t, backendtest.MasterEmail, backendtest.MasterPassword)).Status, 401)
	accountEqual(t, accountLogin(ctx, t, b, "next@example.test", backendtest.MasterPassword).User.ID, master.User.ID)
	if err := h.Clock.Advance(time.Minute); err != nil {
		t.Fatal(err)
	}
	challenge = accountDecode(t, start("later@example.test"), webapi.DecodeEmailChangeStarted).Challenge
	if err := h.Clock.Advance(10 * time.Minute); err != nil {
		t.Fatal(err)
	}
	accountRefusal(t, confirm(challenge.ID, h.Mailbox.Sent()[1].Code), 400, webapi.ErrorCodeInvalidCode)
}

func TestAFailedDeliveryAllowsARetryAndWrongCodesOrANewPasswordEndAChallenge(t *testing.T) {
	ctx := t.Context()
	h := accountHarness(ctx, t)
	h.Config.AccountMail = h.Mailbox
	b, master := accountStart(ctx, t, h)
	start := func() backendtest.Answer {
		return accountRequest(ctx, t, b, "POST", "/api/auth/email", &master, accountCredentials(t, "new@example.test", backendtest.MasterPassword))
	}
	confirm := func(id string, code string) backendtest.Answer {
		return accountRequest(ctx, t, b, "POST", "/api/auth/email/confirm", &master, accountJSON(t, contract.Field{Name: "id", Value: id}, contract.Field{Name: "code", Value: code}))
	}
	h.Mailbox.SetFailing(true)
	accountRefusal(t, start(), 503, webapi.ErrorCodeMailFailed)
	h.Mailbox.SetFailing(false)
	challenge := accountDecode(t, start(), webapi.DecodeEmailChangeStarted).Challenge
	code := h.Mailbox.Sent()[0].Code
	wrong := "000000"
	if code == wrong {
		wrong = "111111"
	}
	for range 5 {
		accountRefusal(t, confirm(challenge.ID, wrong), 400, webapi.ErrorCodeInvalidCode)
	}
	accountRefusal(t, confirm(challenge.ID, code), 400, webapi.ErrorCodeInvalidCode)
	if err := h.Clock.Advance(time.Minute); err != nil {
		t.Fatal(err)
	}
	challenge = accountDecode(t, start(), webapi.DecodeEmailChangeStarted).Challenge
	code = h.Mailbox.Sent()[1].Code
	accountEqual(t, accountRequest(ctx, t, b, "PUT", "/api/auth/password", &master, `{"current":"master-pass-1","next":"new-password-2"}`).Status, 204)
	accountRefusal(t, confirm(challenge.ID, code), 400, webapi.ErrorCodeInvalidCode)
}

func TestARequestThatCouldActComesFromAPageOfTheProductOrFromNoPage(t *testing.T) {
	ctx := t.Context()
	h := accountHarness(ctx, t)
	domain, err := expose.ParseDomain("expose.localhost")
	if err != nil {
		t.Fatal(err)
	}
	h.Config.ExposeDomain = &domain
	b, err := h.Start(ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	product := b.URL
	exposed := "http://a1b2c3d4e5.expose.localhost:" + strings.Split(b.URL, ":")[2]
	others := []string{"https://elsewhere.example", exposed, "null"}
	post := func(origin, path string, s *backendtest.Session, body string) backendtest.Answer {
		headers := http.Header{"Content-Type": []string{"text/plain"}}
		if origin != "" {
			headers.Set("Origin", origin)
		}
		r, err := b.Response(ctx, "POST", path, s, headers, strings.NewReader(body))
		if err != nil {
			t.Fatal(err)
		}
		a, err := backendtest.ReadAnswer(ctx, r)
		if err != nil {
			t.Fatal(err)
		}
		return a
	}
	credentials := accountCredentials(t, backendtest.MasterEmail, backendtest.MasterPassword)
	for _, origin := range others {
		a := post(origin, "/api/setup", nil, credentials)
		accountRefusal(t, a, 403, webapi.ErrorCodeForbiddenOrigin)
		accountEqual(t, len(a.SessionCookies()), 0)
	}
	accountEqual(t, accountDecode(t, accountRequest(ctx, t, b, "GET", "/api/setup", nil, ""), webapi.DecodeSetupStatus).Needed, true)
	master, err := backendtest.SessionFrom(post("", "/api/setup", nil, credentials))
	if err != nil {
		t.Fatal(err)
	}
	login := post(exposed, "/api/auth/login", nil, credentials)
	accountRefusal(t, login, 403, webapi.ErrorCodeForbiddenOrigin)
	accountEqual(t, len(login.SessionCookies()), 0)
	accountEqual(t, post(product, "/api/auth/login", nil, credentials).Status, 200)
	body := func(email string) string {
		return accountJSON(t, contract.Field{Name: "email", Value: email}, contract.Field{Name: "password", Value: "user-pass-1"}, contract.Field{Name: "role", Value: "user"})
	}
	for _, origin := range others {
		accountRefusal(t, post(origin, "/api/users", &master, body("forged@example.test")), 403, webapi.ErrorCodeForbiddenOrigin)
		accountRefusal(t, post(origin, "/api/auth/logout", &master, ""), 403, webapi.ErrorCodeForbiddenOrigin)
	}
	users := accountDecode(t, accountRequest(ctx, t, b, "GET", "/api/users", &master, ""), webapi.DecodeUsers).Users
	var emails []string
	for _, u := range users {
		emails = append(emails, string(u.Email))
	}
	accountEqual(t, emails, []string{backendtest.MasterEmail})
	accountEqual(t, post(product, "/api/users", &master, body("page@example.test")).Status, 201)
	accountEqual(t, post("", "/api/users", &master, body("curl@example.test")).Status, 201)
	accountEqual(t, post(product, "/api/auth/logout", &master, "").Status, 204)
}

func TestARequestWhoseOriginAProxyDroppedPassesAndTheLogSaysSoOnce(t *testing.T) {
	ctx := t.Context()
	path := filepath.Join(t.TempDir(), "backend.log")
	file, err := os.Create(path)
	if err != nil {
		t.Fatal(err)
	}
	previous := slog.Default()
	slog.SetDefault(slog.New(slog.NewTextHandler(file, nil)))
	defer func() {
		slog.SetDefault(previous)
		if err := file.Close(); err != nil {
			t.Error(err)
		}
	}()
	h := accountHarness(ctx, t)
	b, err := h.Start(ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	warnings := func() int {
		data, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		count := 0
		for _, line := range strings.Split(string(data), "\n") {
			if strings.Contains(line, "WARN") && strings.Contains(line, "drops the Origin header") {
				count++
			}
		}
		return count
	}
	login := func(site string) backendtest.Answer {
		headers := http.Header{}
		if site != "" {
			headers.Set("Sec-Fetch-Site", site)
		}
		r, err := b.Response(ctx, "POST", "/api/auth/login", nil, headers, strings.NewReader("{}"))
		if err != nil {
			t.Fatal(err)
		}
		a, err := backendtest.ReadAnswer(ctx, r)
		if err != nil {
			t.Fatal(err)
		}
		return a
	}
	accountRefusal(t, login(""), 400, webapi.ErrorCodeInvalidBody)
	accountEqual(t, warnings(), 0)
	for range 2 {
		accountRefusal(t, login("same-origin"), 400, webapi.ErrorCodeInvalidBody)
	}
	accountEqual(t, warnings(), 1)
	if err := b.Close(ctx); err != nil {
		t.Fatal(err)
	}
}
