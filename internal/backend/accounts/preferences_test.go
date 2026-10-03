package accounts

import (
	"context"
	"errors"
	"os"
	"strings"
	"testing"

	"github.com/google/go-cmp/cmp"
	"github.com/wspl/demi/internal/backend/database/databasetest"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/core"
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
	patch *webapi.PreferencesPatch
}

func (*preferenceStore) Preferences(context.Context, webapi.UserID) (webapi.Preferences, error) {
	return webapi.Preferences{}, nil
}

func (s *preferenceStore) PatchPreferences(
	_ context.Context,
	_ webapi.UserID,
	patch webapi.PreferencesPatch,
) (webapi.Preferences, error) {
	s.patch = &patch
	return webapi.Preferences{Locale: patch.Locale}, nil
}

func TestPreferencePatchCanonicalizesBeforeStorage(t *testing.T) {
	control := databasetest.Control(t.Context(), t, core.SystemClock{})
	user := databasetest.Master(t.Context(), t, control)
	service := NewPreferences(control)
	initial, err := webapi.DecodePreferencesPatch(
		[]byte(`{"appearance":{"theme":"dark"},"shortcuts":{"new":"N","sidebar":"S"}}`),
	)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := service.Patch(t.Context(), user.ID, initial); err != nil {
		t.Fatal(err)
	}
	patch, err := webapi.DecodePreferencesPatch(
		[]byte(
			`{"locale":{"timeZone":"asia/shanghai","languages":["zh-cn","EN","zh-CN","iw"]},"shortcuts":{"new":null}}`,
		),
	)
	if err != nil {
		t.Fatal(err)
	}
	saved, err := service.Patch(t.Context(), user.ID, patch)
	if err != nil {
		t.Fatal(err)
	}
	read, err := service.Read(t.Context(), user.ID)
	if err != nil {
		t.Fatal(err)
	}
	if diff := cmp.Diff(saved, read); diff != "" {
		t.Fatal(diff)
	}
	want := commandwire.CommandLocale{
		TimeZone:  "Asia/Shanghai",
		Languages: []commandwire.LanguageTag{"zh-CN", "en", "he"},
	}
	if diff := cmp.Diff(&want, read.Locale); diff != "" {
		t.Fatal(diff)
	}
	if read.Shortcuts.New != nil || read.Shortcuts.Sidebar == nil || *read.Shortcuts.Sidebar != "S" ||
		read.Appearance.Theme == nil ||
		*read.Appearance.Theme != webapi.ThemeDark {
		t.Fatalf("patch did not preserve omitted fields or remove explicit null: %+v", read)
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
	for _, scenario := range cases {
		store := &preferenceStore{}
		_, err := NewPreferences(
			store,
		).Patch(
			t.Context(),
			"caller",
			webapi.PreferencesPatch{
				Locale: &commandwire.CommandLocale{TimeZone: scenario.zone, Languages: scenario.tags},
			},
		)
		if err == nil || store.patch != nil {
			t.Fatal("invalid locale reached storage")
		}
	}
	_, err := Check(
		webapi.PreferencesPatch{
			Locale: &commandwire.CommandLocale{TimeZone: "wrong", Languages: []commandwire.LanguageTag{"en"}},
		},
	)
	var zoneErr *UnknownTimeZone
	if !errors.As(err, &zoneErr) {
		t.Fatalf("wrong error: %v", err)
	}
}
