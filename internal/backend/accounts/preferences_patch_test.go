package accounts_test

import (
	"context"
	"testing"

	"github.com/google/go-cmp/cmp"
	"github.com/wspl/demi/internal/backend/accounts"
	"github.com/wspl/demi/internal/backend/database/databasetest"
	"github.com/wspl/demi/internal/cmdproto"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

type preferenceStore struct {
	patch *webapiproto.PreferencesPatch
}

func (*preferenceStore) Preferences(context.Context, webapiproto.UserID) (webapiproto.Preferences, error) {
	return webapiproto.Preferences{}, nil
}

func (s *preferenceStore) PatchPreferences(
	_ context.Context,
	_ webapiproto.UserID,
	patch webapiproto.PreferencesPatch,
) (webapiproto.Preferences, error) {
	s.patch = &patch
	return webapiproto.Preferences{Locale: patch.Locale}, nil
}

func TestPreferencePatchCanonicalizesBeforeStorage(t *testing.T) {
	control := databasetest.Control(t.Context(), t, types.SystemClock{})
	user := databasetest.Master(t.Context(), t, control)
	service := accounts.NewPreferences(control)
	initial, err := webapiproto.DecodePreferencesPatch(
		[]byte(`{"appearance":{"theme":"dark"},"shortcuts":{"new":"N","sidebar":"S"}}`),
	)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := service.Patch(t.Context(), user.ID, initial); err != nil {
		t.Fatal(err)
	}
	patch, err := webapiproto.DecodePreferencesPatch(
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
	want := cmdproto.CommandLocale{
		TimeZone:  "Asia/Shanghai",
		Languages: []cmdproto.LanguageTag{"zh-CN", "en", "he"},
	}
	if diff := cmp.Diff(&want, read.Locale); diff != "" {
		t.Fatal(diff)
	}
	if read.Shortcuts.New != nil || read.Shortcuts.Sidebar == nil || *read.Shortcuts.Sidebar != "S" ||
		read.Appearance.Theme == nil ||
		*read.Appearance.Theme != webapiproto.ThemeDark {
		t.Fatalf("patch did not preserve omitted fields or remove explicit null: %+v", read)
	}
	if patch.Locale.TimeZone != "asia/shanghai" || len(patch.Locale.Languages) != 4 {
		t.Fatal("mutated caller input")
	}
}

func TestInvalidLocaleDoesNotReachStorage(t *testing.T) {
	cases := []struct {
		zone string
		tags []cmdproto.LanguageTag
	}{
		{"unknown", []cmdproto.LanguageTag{"en"}},
		{"Asia/Kolkata", []cmdproto.LanguageTag{"en"}},
		{"UTC", []cmdproto.LanguageTag{"en--US"}},
		{"UTC", make([]cmdproto.LanguageTag, 17)},
	}
	for _, scenario := range cases {
		store := &preferenceStore{}
		_, err := accounts.NewPreferences(
			store,
		).Patch(
			t.Context(),
			"caller",
			webapiproto.PreferencesPatch{
				Locale: &cmdproto.CommandLocale{TimeZone: scenario.zone, Languages: scenario.tags},
			},
		)
		if err == nil || store.patch != nil {
			t.Fatal("invalid locale reached storage")
		}
	}
	_, err := accounts.Check(
		webapiproto.PreferencesPatch{
			Locale: &cmdproto.CommandLocale{TimeZone: "wrong", Languages: []cmdproto.LanguageTag{"en"}},
		},
	)
	if err == nil || err.Error() != "locale.timeZone: \"wrong\" is not a time zone the backend knows" {
		t.Fatalf("wrong error: %v", err)
	}
}
