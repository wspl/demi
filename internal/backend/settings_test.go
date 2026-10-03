package backend_test

import (
	"strings"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/webapi"
)

// These HTTP scenarios use local databases and no runner or model.
func TestInstanceModeReadBackAsConfigured(t *testing.T) {
	for _, mode := range []webapi.InstanceMode{webapi.InstanceModeShared, webapi.InstanceModeIsolated} {
		t.Run(string(mode), func(t *testing.T) {
			ctx, h := conversationHarness(t)
			h.Config.Mode = mode
			b, s, err := h.StartSetUp(ctx, t)
			wireMust(t, err)
			got := conversationDecode(t, conversationRequest(ctx, t, b, &s, "GET", "/api/settings", "", 200), webapi.DecodeSettings)
			conversationEqual(t, got, webapi.Settings{Mode: mode})
			conversationRefusal(t, conversationRequest(ctx, t, b, nil, "GET", "/api/settings", "", 401), webapi.ErrorCodeUnauthenticated)
		})
	}
}

func TestPreferencePatchesMergeValidateAndSurviveRestart(t *testing.T) {
	ctx, h := conversationHarness(t)
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	device, err := b.Login(ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	const path = "/api/settings/preferences"
	read := func(session *backendtest.Session) webapi.UserPreferences {
		return conversationDecode(t, conversationRequest(ctx, t, b, session, "GET", path, "", 200), webapi.DecodeUserPreferences)
	}
	decode := func(body string) webapi.UserPreferences {
		p, err := webapi.DecodeUserPreferences([]byte(body))
		wireMust(t, err)
		return p
	}
	conversationEqual(t, read(&s), decode(`{"preferences":{"appearance":{},"shortcuts":{}}}`))
	model := `{"providerId":"codex-account","modelId":"chosen-model","thinkingEffort":"high","serviceTierId":null}`
	bodies := []string{`{"appearance":{"theme":"dark"}}`, `{"shortcuts":{"new":"⌘⇧N"}}`, `{"appearance":{"fontSize":17}}`, `{"lastModel":` + model + `}`}
	answers := make([]backendtest.Answer, 4)
	errs := make([]error, 4)
	var workers sync.WaitGroup
	for i, body := range bodies {
		workers.Go(func() {
			session := &device
			if i == 0 {
				session = &s
			}
			answers[i], errs[i] = b.Patch(ctx, path, session, []byte(body))
		})
	}
	workers.Wait()
	for i, a := range answers {
		wireMust(t, errs[i])
		if a.Status != 200 {
			t.Fatalf("patch: %d %s", a.Status, a.Body)
		}
	}
	expected := decode(`{"preferences":{"appearance":{"theme":"dark","fontSize":17},"shortcuts":{"new":"⌘⇧N"},"lastModel":` + model + `}}`)
	conversationEqual(t, read(&device), expected)
	for _, body := range []string{
		`{"appearance":{"fontSize":100}}`, `{"appearance":{"theme":"sepia"}}`, `{"appearance":null}`,
		`{"shortcuts":{"arbitrary":"x"}}`, `{"shortcuts":{"new":"` + strings.Repeat("x", 65) + `"}}`,
		`{"lastModel":{"providerId":"","modelId":"chosen-model","thinkingEffort":null,"serviceTierId":null}}`,
		`{"lastModel":{"providerId":"codex-account","modelId":"chosen-model"}}`, `{"remember":true}`,
	} {
		conversationRefusal(t, conversationRequest(ctx, t, b, &device, "PATCH", path, body, 400), webapi.ErrorCodeInvalidBody)
	}
	wireMust(t, b.Close(ctx))
	b, err = h.Start(ctx, t)
	wireMust(t, err)
	conversationEqual(t, read(&s), expected)
	removed := conversationDecode(t, conversationRequest(ctx, t, b, &s, "PATCH", path, `{"shortcuts":{"new":null}}`, 200), webapi.DecodeUserPreferences)
	expected.Preferences.Shortcuts = webapi.Shortcuts{}
	conversationEqual(t, removed, expected)
}

func TestReportedLocaleValidatedAndCanonicalized(t *testing.T) {
	ctx, h := conversationHarness(t)
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	for _, test := range []struct{ locale, field string }{
		{`{"timeZone":"Mars/Olympus_Mons","languages":["en"]}`, "locale.timeZone"},
		{`{"timeZone":"UTC","languages":["en","en_US"]}`, "locale.languages[1]"},
		{`{"timeZone":"UTC","languages":[]}`, "languages"},
		{`{"timeZone":"UTC","languages":[` + strings.Repeat(`"en",`, 16) + `"en"]}`, "languages"},
		{`{"timeZone":"UTC"}`, "languages"},
	} {
		a := conversationRequest(ctx, t, b, &s, "PATCH", "/api/settings/preferences", `{"locale":`+test.locale+`}`, 400)
		conversationRefusal(t, a, webapi.ErrorCodeInvalidBody)
		e, err := a.ErrorBody()
		wireMust(t, err)
		if !strings.Contains(e.Message, test.field) {
			t.Fatalf("%s: %s", test.locale, e.Message)
		}
	}
	a := conversationRequest(ctx, t, b, &s, "PATCH", "/api/settings/preferences", `{"locale":{"timeZone":"asia/shanghai","languages":["zh-cn","EN","zh-CN","iw"]}}`, 200)
	p := conversationDecode(t, a, webapi.DecodeUserPreferences)
	conversationEqual(t, p.Preferences.Locale.TimeZone, "Asia/Shanghai")
	conversationEqual(t, p.Preferences.Locale.Languages, []commandwire.LanguageTag{"zh-CN", "en", "he"})
}
