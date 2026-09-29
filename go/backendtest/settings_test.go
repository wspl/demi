package backendtest_test

import (
	"net/http"
	"strings"
	"testing"

	"github.com/wspl/demi/go/backendtest"
)

const preferencesPath = "/api/settings/preferences"

// Cost: two backends, about a second each.
func TestTheInstanceModeIsReadBackAsConfigured(t *testing.T) {
	t.Parallel()
	for _, mode := range []string{"shared", "isolated"} {
		t.Run(mode, func(t *testing.T) {
			t.Parallel()
			b, master := backendtest.New(t, backendtest.WithMode(mode)).StartSetUp()
			backendtest.AssertJSON(t, b.Get("/api/settings", master).Expect(http.StatusOK).Value(), backendtest.Map{"mode": mode})
			anonymous := b.Get("/api/settings", nil)
			if status, code := anonymous.Refusal(); status != http.StatusUnauthorized || code != "unauthenticated" {
				t.Fatalf("settings without a session are %d %s", status, code)
			}
			b.Stop()
		})
	}
}

// Cost: one backend started twice, about two seconds.
func TestPreferencePatchesMergeFieldByFieldRefuseWhatIsInvalidAndSurviveARestart(t *testing.T) {
	t.Parallel()
	h := backendtest.New(t)
	b, master := h.StartSetUp()
	device := b.Login(backendtest.MasterEmail, backendtest.MasterPassword)
	saved := func(session *backendtest.Session) any {
		return b.Get(preferencesPath, session).Expect(http.StatusOK).Value()
	}
	backendtest.AssertJSON(t, saved(master), backendtest.Map{"preferences": backendtest.Map{
		"appearance": backendtest.Map{}, "shortcuts": backendtest.Map{},
	}})

	lastModel := backendtest.Map{
		"providerId": "codex-account", "modelId": "chosen-model", "thinkingEffort": "high", "serviceTierId": nil,
	}
	patches := []struct {
		session *backendtest.Session
		body    backendtest.Map
	}{
		{master, backendtest.Map{"appearance": backendtest.Map{"theme": "dark"}}},
		{device, backendtest.Map{"shortcuts": backendtest.Map{"new": "⌘⇧N"}}},
		{device, backendtest.Map{"appearance": backendtest.Map{"fontSize": 17}}},
		{device, backendtest.Map{"lastModel": lastModel}},
	}
	statuses := make([]int, len(patches))
	backendtest.Concurrently(len(patches), func(index int) {
		answer, err := b.TryDo(backendtest.Request{
			Method: http.MethodPatch, Path: preferencesPath, Session: patches[index].session, Body: patches[index].body,
		})
		if err != nil {
			t.Error(err)
			return
		}
		statuses[index] = answer.Status
	})
	for index, status := range statuses {
		if status != http.StatusOK {
			t.Fatalf("patch %d answers %d", index, status)
		}
	}
	expected := backendtest.Map{"preferences": backendtest.Map{
		"appearance": backendtest.Map{"theme": "dark", "fontSize": 17},
		"shortcuts":  backendtest.Map{"new": "⌘⇧N"},
		"lastModel":  lastModel,
	}}
	backendtest.AssertJSON(t, saved(device), expected)

	for _, refused := range []backendtest.Map{
		{"appearance": backendtest.Map{"fontSize": 100}},
		{"appearance": backendtest.Map{"theme": "sepia"}},
		{"appearance": nil},
		{"shortcuts": backendtest.Map{"arbitrary": "x"}},
		{"shortcuts": backendtest.Map{"new": strings.Repeat("x", 65)}},
		{"lastModel": backendtest.Map{"providerId": "", "modelId": "chosen-model", "thinkingEffort": nil, "serviceTierId": nil}},
		{"lastModel": backendtest.Map{"providerId": "codex-account", "modelId": "chosen-model"}},
		{"remember": true},
	} {
		answer := b.Patch(preferencesPath, device, refused)
		if status, code := answer.Refusal(); status != http.StatusBadRequest || code != "invalid_body" {
			t.Fatalf("%v is %d %s", refused, status, code)
		}
	}

	b.Stop()
	b = h.Start()
	backendtest.AssertJSON(t, saved(master), expected)
	// A null shortcut removes that override and leaves the rest.
	removed := b.Patch(preferencesPath, master, backendtest.Map{"shortcuts": backendtest.Map{"new": nil}})
	backendtest.AssertJSON(t, removed.Expect(http.StatusOK).Value(), backendtest.Map{"preferences": backendtest.Map{
		"appearance": backendtest.Map{"theme": "dark", "fontSize": 17},
		"shortcuts":  backendtest.Map{},
		"lastModel":  lastModel,
	}})
	b.Stop()
}

// Cost: one backend, about a second.
func TestAReportedLocaleIsCheckedAndKeptInCanonicalForm(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()
	sixteen := make([]string, 17)
	for i := range sixteen {
		sixteen[i] = "en"
	}
	for _, refusal := range []struct {
		locale backendtest.Map
		field  string
	}{
		{backendtest.Map{"timeZone": "Mars/Olympus_Mons", "languages": []string{"en"}}, "locale.timeZone"},
		{backendtest.Map{"timeZone": "UTC", "languages": []string{"en", "en_US"}}, "locale.languages[1]"},
		{backendtest.Map{"timeZone": "UTC", "languages": []string{}}, "languages"},
		{backendtest.Map{"timeZone": "UTC", "languages": sixteen}, "languages"},
		{backendtest.Map{"timeZone": "UTC"}, "languages"},
	} {
		answer := b.Patch(preferencesPath, master, backendtest.Map{"locale": refusal.locale})
		if status, code := answer.Refusal(); status != http.StatusBadRequest || code != "invalid_body" {
			t.Fatalf("%v is %d %s", refusal.locale, status, code)
		}
		if !strings.Contains(answer.ErrorMessage(), refusal.field) {
			t.Fatalf("%v: the message %q does not name %q", refusal.locale, answer.ErrorMessage(), refusal.field)
		}
	}

	reported := backendtest.Map{"timeZone": "asia/shanghai", "languages": []string{"zh-cn", "EN", "zh-CN", "iw"}}
	answer := b.Patch(preferencesPath, master, backendtest.Map{"locale": reported}).Expect(http.StatusOK)
	backendtest.AssertJSON(t, answer.At("preferences.locale"), backendtest.Map{
		"timeZone": "Asia/Shanghai", "languages": []string{"zh-CN", "en", "he"},
	})
	b.Stop()
}
