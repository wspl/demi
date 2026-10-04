package scenarios_test

import (
	"log/slog"
	"net/http"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/webapiproto"
)

// accountJSON encodes fields in their given order, escaping strings as the wire contract does (no HTML escaping).
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
	return accountJSON(
		t,
		contract.Field{Name: "email", Value: email},
		contract.Field{Name: "password", Value: password},
	)
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

// TestSetupCreatesTheMasterOnceAndSignsItIn
// checks first-use setup and the resulting master session.
func TestSetupCreatesTheMasterOnceAndSignsItIn(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	server, err := harness.Start(ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	conversationEqual(
		t,
		conversationDecode(
			t,
			conversationRequest(ctx, t, server, nil, "GET", "/api/setup", "", 200),
			webapiproto.DecodeSetupStatus,
		).Needed,
		true,
	)
	a := conversationRequest(
		ctx,
		t,
		server,
		nil,
		"POST",
		"/api/setup",
		accountCredentials(t, "  Master@Example.TEST ", backendtest.MasterPassword),
		201,
	)
	master, err := backendtest.SessionFrom(a)
	if err != nil {
		t.Fatal(err)
	}
	conversationEqual(t, string(master.User.Email), backendtest.MasterEmail)
	conversationEqual(t, master.User.Role, webapiproto.RoleMaster)
	conversationEqual(t, master.User.Nickname, "")
	token := strings.TrimPrefix(master.Cookie, backendtest.SessionCookie+"=")
	if len(token) < 40 ||
		strings.ContainsFunc(
			token,
			func(r rune) bool {
				return (r < 'a' || r > 'z') && (r < 'A' || r > 'Z') && (r < '0' || r > '9')
			},
		) {
		t.Fatalf("token: %q", token)
	}
	for _, attr := range []string{"httponly", "samesite=lax", "path=/", "expires=sat, 24 oct 2026 08:00:00 gmt"} {
		accountCookie(t, a, attr)
	}
	if strings.Contains(strings.ToLower(a.SessionCookies()[0]), "; secure") {
		t.Fatal("HTTP cookie is secure")
	}
	conversationEqual(
		t,
		conversationDecode(
			t,
			conversationRequest(ctx, t, server, nil, "GET", "/api/setup", "", 200),
			webapiproto.DecodeSetupStatus,
		).Needed,
		false,
	)
	again := conversationRequest(
		ctx,
		t,
		server,
		nil,
		"POST",
		"/api/setup",
		accountCredentials(t, "other@example.test", "other-pass-1"),
		404,
	)
	conversationRefusal(t, again, webapiproto.ErrorCodeAlreadySetUp)
	conversationEqual(t, len(again.SessionCookies()), 0)
	conversationEqual(
		t,
		conversationDecode(
			t,
			conversationRequest(ctx, t, server, &master, "GET", "/api/auth/me", "", 200),
			webapiproto.DecodeIdentity,
		).User,
		master.User,
	)
}

// TestConcurrentSetupsCreateOneMaster
// checks that concurrent setup requests publish one master.
func TestConcurrentSetupsCreateOneMaster(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	server, err := harness.Start(ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	bodies := []string{
		accountCredentials(t, "first@example.test", backendtest.MasterPassword),
		accountCredentials(t, "second@example.test", backendtest.MasterPassword),
	}
	answers := make([]backendtest.Answer, 2)
	errs := make([]error, 2)
	var wg sync.WaitGroup
	for i := range bodies {
		wg.Go(func() {
			answers[i], errs[i] = server.Post(ctx, "/api/setup", nil, []byte(bodies[i]))
		})
	}
	wg.Wait()
	for _, err := range errs {
		if err != nil {
			t.Fatal(err)
		}
	}
	statuses := []int{answers[0].Status, answers[1].Status}
	slices.Sort(statuses)
	conversationEqual(t, statuses, []int{201, 404})
}

// TestEveryOtherAPIPathWantsALiveSession
// checks authentication before routing protected API requests.
func TestEveryOtherAPIPathWantsALiveSession(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	server, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	for _, route := range [][2]string{
		{"GET", "/api/auth/me"},
		{"GET", "/api/no-such-resource"},
		{"GET", "/api"},
		{"GET", "/api/setup/more"},
		{"DELETE", "/api/auth/me"},
	} {
		a := conversationRequest(ctx, t, server, nil, route[0], route[1], "", 401)
		conversationRefusal(t, a, webapiproto.ErrorCodeUnauthenticated)
		conversationEqual(t, len(a.SessionCookies()), 0)
	}
	eAnswer := conversationRequest(ctx, t, server, &master, "GET", "/api/no-such-resource", "", 404)
	e := conversationDecode(t, eAnswer, webapiproto.DecodeErrorBody)
	conversationEqual(t, e.Code, webapiproto.ErrorCodeNotFound)
	conversationEqual(t, e.Message, "No route for GET /api/no-such-resource")
	conversationRefusal(
		t,
		conversationRequest(ctx, t, server, &master, "DELETE", "/api/auth/me", "", 404),
		webapiproto.ErrorCodeNotFound,
	)
	forged := backendtest.Session{Cookie: backendtest.SessionCookie + "=not-a-session"}
	a := conversationRequest(ctx, t, server, &forged, "GET", "/api/auth/me", "", 401)
	conversationRefusal(t, a, webapiproto.ErrorCodeUnauthenticated)
	accountCookie(t, a, backendtest.SessionCookie+"=")
	accountCookie(t, a, "max-age=0")
}

// TestARequestOverHTTPSGetsASecureCookie
// checks the cookie attributes on an HTTPS deployment.
func TestARequestOverHTTPSGetsASecureCookie(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	server, _, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	r, err := server.Response(
		ctx,
		"POST",
		"/api/auth/login",
		nil,
		http.Header{"X-Forwarded-Proto": []string{"https, http"}},
		strings.NewReader(accountCredentials(t, backendtest.MasterEmail, backendtest.MasterPassword)),
	)
	if err != nil {
		t.Fatal(err)
	}
	a, err := backendtest.ReadAnswer(ctx, r)
	if err != nil {
		t.Fatal(err)
	}
	conversationEqual(t, a.Status, 200)
	accountCookie(t, a, "secure")
}

// TestLoginLocksOutAfterFiveFailuresAndLogoutEndsTheSession
// checks lockout and session revocation through HTTP.
func TestLoginLocksOutAfterFiveFailuresAndLogoutEndsTheSession(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	server, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	login := func(password string) backendtest.Answer {
		answer, err := server.Post(
			ctx,
			"/api/auth/login",
			nil,
			[]byte(accountCredentials(t, backendtest.MasterEmail, password)),
		)
		wireMust(t, err)
		return answer
	}
	wrong := login("nope-nope")
	conversationEqual(t, wrong.Status, 401)
	conversationRefusal(t, wrong, webapiproto.ErrorCodeInvalidCredentials)
	conversationEqual(t, len(wrong.SessionCookies()), 0)
	signed, err := server.Login(ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	conversationEqual(t, signed.User.ID, master.User.ID)
	if signed.Cookie == master.Cookie {
		t.Fatal("login reused cookie")
	}
	for range 5 {
		conversationEqual(t, login("nope-nope").Status, 401)
	}
	{
		answer := login(backendtest.MasterPassword)
		conversationEqual(t, answer.Status, 429)
		conversationRefusal(t, answer, webapiproto.ErrorCodeTooManyAttempts)
	}
	conversationRequest(ctx, t, server, &signed, "GET", "/api/auth/me", "", 200)
	out := conversationRequest(ctx, t, server, &signed, "POST", "/api/auth/logout", "{}", 204)
	accountCookie(t, out, "max-age=0")
	conversationRequest(ctx, t, server, &signed, "GET", "/api/auth/me", "", 401)
	conversationRequest(ctx, t, server, &master, "GET", "/api/auth/me", "", 200)
}

// TestAnAddressWithoutAnAccountIsLockedOutToo
// checks that unknown addresses follow the same lockout policy.
func TestAnAddressWithoutAnAccountIsLockedOutToo(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	server, _, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	for range 5 {
		conversationRefusal(
			t,
			conversationRequest(
				ctx,
				t,
				server,
				nil,
				"POST",
				"/api/auth/login",
				accountCredentials(t, "nobody@example.test", "nope-nope"),
				401,
			),
			webapiproto.ErrorCodeInvalidCredentials,
		)
	}
	conversationRefusal(
		t,
		conversationRequest(
			ctx,
			t,
			server,
			nil,
			"POST",
			"/api/auth/login",
			accountCredentials(t, "NOBODY@example.test", "nope-nope"),
			429,
		),
		webapiproto.ErrorCodeTooManyAttempts,
	)
	_, err = server.Login(ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
}

// TestAUserChangesTheirOwnPasswordWithTheCurrentOne
// checks password changes and subsequent login credentials.
func TestAUserChangesTheirOwnPasswordWithTheCurrentOne(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	server, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	change := func(current, next string) backendtest.Answer {
		answer, err := server.Put(
			ctx,
			"/api/auth/password",
			&master,
			[]byte(
				accountJSON(
					t,
					contract.Field{Name: "current", Value: current},
					contract.Field{Name: "next", Value: next},
				),
			),
		)
		wireMust(t, err)
		return answer
	}
	{
		answer := change("wrong-wrong", "second-pass-2")
		conversationEqual(t, answer.Status, 401)
		conversationRefusal(t, answer, webapiproto.ErrorCodeInvalidCredentials)
	}
	eAnswer := change(backendtest.MasterPassword, "short")
	conversationEqual(t, eAnswer.Status, 400)
	e := conversationDecode(t, eAnswer, webapiproto.DecodeErrorBody)
	conversationEqual(t, e.Code, webapiproto.ErrorCodeInvalidBody)
	if !strings.Contains(e.Message, "next") {
		t.Fatal(e.Message)
	}
	conversationEqual(t, change(backendtest.MasterPassword, "second-pass-2").Status, 204)
	conversationRequest(
		ctx,
		t,
		server,
		nil,
		"POST",
		"/api/auth/login",
		accountCredentials(t, backendtest.MasterEmail, backendtest.MasterPassword),
		401,
	)
	signed1, err := server.Login(ctx, backendtest.MasterEmail, "second-pass-2")
	wireMust(t, err)
	conversationEqual(t, signed1.User.ID, master.User.ID)
}

// TestASessionSlidesWhileUsedAndExpiresWhenSilent
// checks activity-based session renewal and expiry.
func TestASessionSlidesWhileUsedAndExpiresWhenSilent(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	server, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	advance := func(days int) {
		if err := harness.Clock.Advance(time.Duration(days) * 24 * time.Hour); err != nil {
			t.Fatal(err)
		}
	}
	advance(10)
	early := conversationRequest(ctx, t, server, &master, "GET", "/api/auth/me", "", 200)
	conversationEqual(t, len(early.SessionCookies()), 0)
	advance(6)
	renewed := conversationRequest(ctx, t, server, &master, "GET", "/api/auth/me", "", 200)
	accountCookie(t, renewed, master.Cookie)
	accountCookie(t, renewed, "expires=mon, 09 nov 2026 08:00:00 gmt")
	advance(20)
	conversationRequest(ctx, t, server, &master, "GET", "/api/auth/me", "", 200)
	advance(31)
	expired := conversationRequest(ctx, t, server, &master, "GET", "/api/auth/me", "", 401)
	conversationRefusal(t, expired, webapiproto.ErrorCodeUnauthenticated)
	accountCookie(t, expired, "max-age=0")
}

// TestEmailIdentityIsNormalizedAndTheNicknamePersists
// checks email identity and stored profile changes.
func TestEmailIdentityIsNormalizedAndTheNicknamePersists(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	server, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	signed, err := server.Login(ctx, "  "+strings.ToUpper(backendtest.MasterEmail)+"  ", backendtest.MasterPassword)
	wireMust(t, err)
	conversationEqual(t, string(signed.User.Email), backendtest.MasterEmail)
	eAnswer := conversationRequest(
		ctx,
		t,
		server,
		nil,
		"POST",
		"/api/auth/login",
		accountCredentials(t, "invalid", backendtest.MasterPassword),
		400,
	)
	e := conversationDecode(t, eAnswer, webapiproto.DecodeErrorBody)
	conversationEqual(t, e.Code, webapiproto.ErrorCodeInvalidBody)
	if !strings.Contains(e.Message, "email") {
		t.Fatal(e.Message)
	}
	user := conversationDecode(
		t,
		conversationRequest(ctx, t, server, &signed, "PATCH", "/api/auth/me", `{"nickname":"  New name  "}`, 200),
		webapiproto.DecodeIdentity,
	).User
	conversationEqual(t, string(user.Email), backendtest.MasterEmail)
	conversationEqual(t, user.Nickname, "New name")
	conversationEqual(
		t,
		conversationDecode(
			t,
			conversationRequest(ctx, t, server, &master, "GET", "/api/auth/me", "", 200),
			webapiproto.DecodeIdentity,
		).User.Nickname,
		"New name",
	)
	for _, body := range []string{`{"nickname":" "}`, `{"nickname":"Ana","role":"admin"}`} {
		conversationRefusal(
			t,
			conversationRequest(ctx, t, server, &signed, "PATCH", "/api/auth/me", body, 400),
			webapiproto.ErrorCodeInvalidBody,
		)
	}
	conversationRefusal(
		t,
		conversationRequest(
			ctx,
			t,
			server,
			&signed,
			"POST",
			"/api/auth/email",
			accountCredentials(t, "next@example.test", backendtest.MasterPassword),
			503,
		),
		webapiproto.ErrorCodeMailUnavailable,
	)
}

// TestAnEmailChangeNeedsADeliveredUnexpiredSingleUseCodeAndSurvivesRestart
// checks email challenges and restart persistence.
func TestAnEmailChangeNeedsADeliveredUnexpiredSingleUseCodeAndSurvivesRestart(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	harness.Config.AccountMail = harness.Mailbox
	server, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	start := func(email string) backendtest.Answer {
		answer, err := server.Post(
			ctx,
			"/api/auth/email",
			&master,
			[]byte(accountCredentials(t, email, backendtest.MasterPassword)),
		)
		wireMust(t, err)
		return answer
	}
	confirm := func(id string, code string) backendtest.Answer {
		answer, err := server.Post(
			ctx,
			"/api/auth/email/confirm",
			&master,
			[]byte(accountJSON(t, contract.Field{Name: "id", Value: id}, contract.Field{Name: "code", Value: code})),
		)
		wireMust(t, err)
		return answer
	}
	conversationRefusal(
		t,
		conversationRequest(
			ctx,
			t,
			server,
			&master,
			"POST",
			"/api/auth/email",
			accountCredentials(t, "next@example.test", "wrong-password"),
			401,
		),
		webapiproto.ErrorCodeInvalidCredentials,
	)
	{
		answer := start(backendtest.MasterEmail)
		conversationEqual(t, answer.Status, 409)
		conversationRefusal(t, answer, webapiproto.ErrorCodeEmailTaken)
	}
	started := start("NEXT@example.test")
	conversationEqual(t, started.Status, 202)
	challenge := conversationDecode(t, started, webapiproto.DecodeEmailChangeStarted).Challenge
	mail := harness.Mailbox.Sent()[0]
	conversationEqual(t, string(mail.Email), "next@example.test")
	conversationEqual(t, challenge.Email, mail.Email)
	conversationEqual(t, string(challenge.ExpiresAt), "2026-09-24T08:10:00.000Z")
	if strings.Contains(string(started.Body), mail.Code) {
		t.Fatal("answer contains code")
	}
	{
		answer := start("next@example.test")
		conversationEqual(t, answer.Status, 429)
		conversationRefusal(t, answer, webapiproto.ErrorCodeTooManyAttempts)
	}
	for _, code := range []string{"abcdef", "١٢٣٤٥٦"} {
		answer := confirm(challenge.ID, code)
		conversationEqual(t, answer.Status, 400)
		conversationRefusal(t, answer, webapiproto.ErrorCodeInvalidBody)
	}
	if err := server.Close(ctx); err != nil {
		t.Fatal(err)
	}
	server, err = harness.Start(ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	master, err = server.Login(ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	confirmed := confirm(challenge.ID, mail.Code)
	conversationEqual(t, confirmed.Status, 200)
	conversationEqual(
		t,
		string(conversationDecode(t, confirmed, webapiproto.DecodeIdentity).User.Email),
		"next@example.test",
	)
	{
		answer := confirm(challenge.ID, mail.Code)
		conversationEqual(t, answer.Status, 400)
		conversationRefusal(t, answer, webapiproto.ErrorCodeInvalidCode)
	}
	conversationRequest(
		ctx,
		t,
		server,
		nil,
		"POST",
		"/api/auth/login",
		accountCredentials(t, backendtest.MasterEmail, backendtest.MasterPassword),
		401,
	)
	signed2, err := server.Login(ctx, "next@example.test", backendtest.MasterPassword)
	wireMust(t, err)
	conversationEqual(t, signed2.User.ID, master.User.ID)
	if err := harness.Clock.Advance(time.Minute); err != nil {
		t.Fatal(err)
	}
	challenge = conversationDecode(t, start("later@example.test"), webapiproto.DecodeEmailChangeStarted).Challenge
	if err := harness.Clock.Advance(10 * time.Minute); err != nil {
		t.Fatal(err)
	}
	answer := confirm(challenge.ID, harness.Mailbox.Sent()[1].Code)
	conversationEqual(t, answer.Status, 400)
	conversationRefusal(t, answer, webapiproto.ErrorCodeInvalidCode)
}

// TestAFailedDeliveryAllowsARetryAndWrongCodesOrANewPasswordEndAChallenge
// checks challenge cancellation and delivery retries.
func TestAFailedDeliveryAllowsARetryAndWrongCodesOrANewPasswordEndAChallenge(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	harness.Config.AccountMail = harness.Mailbox
	server, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	start := func() backendtest.Answer {
		answer, err := server.Post(
			ctx,
			"/api/auth/email",
			&master,
			[]byte(accountCredentials(t, "new@example.test", backendtest.MasterPassword)),
		)
		wireMust(t, err)
		return answer
	}
	confirm := func(id string, code string) backendtest.Answer {
		answer, err := server.Post(
			ctx,
			"/api/auth/email/confirm",
			&master,
			[]byte(accountJSON(t, contract.Field{Name: "id", Value: id}, contract.Field{Name: "code", Value: code})),
		)
		wireMust(t, err)
		return answer
	}
	harness.Mailbox.SetFailing(true)
	{
		answer := start()
		conversationEqual(t, answer.Status, 503)
		conversationRefusal(t, answer, webapiproto.ErrorCodeMailFailed)
	}
	harness.Mailbox.SetFailing(false)
	challenge := conversationDecode(t, start(), webapiproto.DecodeEmailChangeStarted).Challenge
	code := harness.Mailbox.Sent()[0].Code
	wrong := "000000"
	if code == wrong {
		wrong = "111111"
	}
	for range 5 {
		answer := confirm(challenge.ID, wrong)
		conversationEqual(t, answer.Status, 400)
		conversationRefusal(t, answer, webapiproto.ErrorCodeInvalidCode)
	}
	{
		answer := confirm(challenge.ID, code)
		conversationEqual(t, answer.Status, 400)
		conversationRefusal(t, answer, webapiproto.ErrorCodeInvalidCode)
	}
	if err := harness.Clock.Advance(time.Minute); err != nil {
		t.Fatal(err)
	}
	challenge = conversationDecode(t, start(), webapiproto.DecodeEmailChangeStarted).Challenge
	code = harness.Mailbox.Sent()[1].Code
	conversationRequest(
		ctx,
		t,
		server,
		&master,
		"PUT",
		"/api/auth/password",
		`{"current":"master-pass-1","next":"new-password-2"}`,
		204,
	)
	answer := confirm(challenge.ID, code)
	conversationEqual(t, answer.Status, 400)
	conversationRefusal(t, answer, webapiproto.ErrorCodeInvalidCode)
}

// TestARequestThatCouldActComesFromAPageOfTheProductOrFromNoPage
// checks the origin rules for state-changing requests.
func TestARequestThatCouldActComesFromAPageOfTheProductOrFromNoPage(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	domain, err := expose.ParseDomain("expose.localhost")
	if err != nil {
		t.Fatal(err)
	}
	harness.Config.ExposeDomain = &domain
	server, err := harness.Start(ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	product := server.URL
	exposed := "http://a1b2c3d4e5.expose.localhost:" + strings.Split(server.URL, ":")[2]
	others := []string{"https://elsewhere.example", exposed, "null"}
	post := func(origin, path string, s *backendtest.Session, body string) backendtest.Answer {
		headers := http.Header{"Content-Type": []string{"text/plain"}}
		if origin != "" {
			headers.Set("Origin", origin)
		}
		r, err := server.Response(ctx, "POST", path, s, headers, strings.NewReader(body))
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
		conversationEqual(t, a.Status, 403)
		conversationRefusal(t, a, webapiproto.ErrorCodeForbiddenOrigin)
		conversationEqual(t, len(a.SessionCookies()), 0)
	}
	conversationEqual(
		t,
		conversationDecode(
			t,
			conversationRequest(ctx, t, server, nil, "GET", "/api/setup", "", 200),
			webapiproto.DecodeSetupStatus,
		).Needed,
		true,
	)
	master, err := backendtest.SessionFrom(post("", "/api/setup", nil, credentials))
	if err != nil {
		t.Fatal(err)
	}
	login := post(exposed, "/api/auth/login", nil, credentials)
	conversationEqual(t, login.Status, 403)
	conversationRefusal(t, login, webapiproto.ErrorCodeForbiddenOrigin)
	conversationEqual(t, len(login.SessionCookies()), 0)
	conversationEqual(t, post(product, "/api/auth/login", nil, credentials).Status, 200)
	body := func(email string) string {
		return accountJSON(
			t,
			contract.Field{Name: "email", Value: email},
			contract.Field{Name: "password", Value: "user-pass-1"},
			contract.Field{Name: "role", Value: "user"},
		)
	}
	for _, origin := range others {
		{
			answer := post(origin, "/api/users", &master, body("forged@example.test"))
			conversationEqual(t, answer.Status, 403)
			conversationRefusal(t, answer, webapiproto.ErrorCodeForbiddenOrigin)
		}
		answer := post(origin, "/api/auth/logout", &master, "")
		conversationEqual(t, answer.Status, 403)
		conversationRefusal(t, answer, webapiproto.ErrorCodeForbiddenOrigin)
	}
	users := conversationDecode(
		t,
		conversationRequest(ctx, t, server, &master, "GET", "/api/users", "", 200),
		webapiproto.DecodeUsers,
	).Users
	var emails []string
	for _, u := range users {
		emails = append(emails, string(u.Email))
	}
	conversationEqual(t, emails, []string{backendtest.MasterEmail})
	conversationEqual(t, post(product, "/api/users", &master, body("page@example.test")).Status, 201)
	conversationEqual(t, post("", "/api/users", &master, body("curl@example.test")).Status, 201)
	conversationEqual(t, post(product, "/api/auth/logout", &master, "").Status, 204)
}

// TestARequestWhoseOriginAProxyDroppedPassesAndTheLogSaysSoOnce
// checks the missing-origin warning and request acceptance.
func TestARequestWhoseOriginAProxyDroppedPassesAndTheLogSaysSoOnce(t *testing.T) {
	// Serial: this scenario replaces the process-wide slog logger.
	ctx, harness := conversationHarness(t)
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
	server, err := harness.Start(ctx, t)
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
		r, err := server.Response(ctx, "POST", "/api/auth/login", nil, headers, strings.NewReader("{}"))
		if err != nil {
			t.Fatal(err)
		}
		a, err := backendtest.ReadAnswer(ctx, r)
		if err != nil {
			t.Fatal(err)
		}
		return a
	}
	{
		answer := login("")
		conversationEqual(t, answer.Status, 400)
		conversationRefusal(t, answer, webapiproto.ErrorCodeInvalidBody)
	}
	conversationEqual(t, warnings(), 0)
	for range 2 {
		answer := login("same-origin")
		conversationEqual(t, answer.Status, 400)
		conversationRefusal(t, answer, webapiproto.ErrorCodeInvalidBody)
	}
	conversationEqual(t, warnings(), 1)
	if err := server.Close(ctx); err != nil {
		t.Fatal(err)
	}
}
