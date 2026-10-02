package accounts

import (
	"context"
	"errors"
	"os"
	"strings"
	"testing"

	"github.com/google/go-cmp/cmp"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/webapi"
)

func TestRustLocaleFixtures(t *testing.T) {
	data, err := os.ReadFile("testdata/locales.tsv")
	if err != nil {
		t.Fatal(err)
	}
	for _, row := range strings.Split(strings.TrimSpace(string(data)), "\n") {
		fields := strings.Split(row, "\t")
		t.Run(fields[0], func(t *testing.T) {
			got, err := canonicalLanguage(fields[0])
			if fields[1] == "ERROR" {
				if err == nil {
					t.Fatalf("Rust refused %q, Go accepted %q", fields[0], got)
				}
			} else if err != nil || got != fields[1] {
				t.Fatalf("got %q (%v), Rust wrote %q", got, err, fields[1])
			}
		})
	}
}

type preferenceStore struct {
	caller webapi.UserID
	patch  *webapi.PreferencesPatch
}

func (*preferenceStore) Preferences(context.Context, webapi.UserID) (webapi.Preferences, error) {
	return webapi.Preferences{}, nil
}
func (s *preferenceStore) PatchPreferences(_ context.Context, caller webapi.UserID, patch webapi.PreferencesPatch) (webapi.Preferences, error) {
	s.caller = caller
	s.patch = &patch
	return webapi.Preferences{Locale: patch.Locale}, nil
}

func TestPreferencePatchCanonicalizesBeforeStorage(t *testing.T) {
	store := &preferenceStore{}
	service := NewPreferences(store)
	patch, err := webapi.DecodePreferencesPatch([]byte(`{"locale":{"timeZone":"asia/shanghai","languages":["zh-cn","EN","zh-CN","iw"]},"shortcuts":{"new":null}}`))
	if err != nil {
		t.Fatal(err)
	}
	_, err = service.Patch(t.Context(), "caller", patch)
	if err != nil {
		t.Fatal(err)
	}
	want := commandwire.CommandLocale{TimeZone: "Asia/Shanghai", Languages: []commandwire.LanguageTag{"zh-CN", "en", "he"}}
	if diff := cmp.Diff(want, *store.patch.Locale); diff != "" {
		t.Fatal(diff)
	}
	if store.caller != "caller" || store.patch.Shortcuts.New == nil || *store.patch.Shortcuts.New != nil {
		t.Fatal("patch lost caller or explicit null")
	}
	if patch.Locale.TimeZone != "asia/shanghai" || len(patch.Locale.Languages) != 4 {
		t.Fatal("mutated caller input")
	}
}

func TestInvalidLocaleDoesNotReachStorage(t *testing.T) {
	cases := []struct {
		zone string
		tags []commandwire.LanguageTag
	}{
		{"unknown", []commandwire.LanguageTag{"en"}},
		{"Asia/Kolkata", []commandwire.LanguageTag{"en"}},
		{"UTC", []commandwire.LanguageTag{"en--US"}},
		{"UTC", make([]commandwire.LanguageTag, 17)},
	}
	for _, tc := range cases {
		store := &preferenceStore{}
		_, err := NewPreferences(store).Patch(t.Context(), "caller", webapi.PreferencesPatch{Locale: &commandwire.CommandLocale{TimeZone: tc.zone, Languages: tc.tags}})
		if err == nil || store.patch != nil {
			t.Fatal("invalid locale reached storage")
		}
	}
	_, err := Check(webapi.PreferencesPatch{Locale: &commandwire.CommandLocale{TimeZone: "wrong", Languages: []commandwire.LanguageTag{"en"}}})
	var zoneErr *UnknownTimeZone
	if !errors.As(err, &zoneErr) {
		t.Fatalf("wrong error: %v", err)
	}
}
