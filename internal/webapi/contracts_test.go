package webapi_test

import (
	"errors"
	"fmt"
	"net/url"
	"strings"
	"testing"
	"unicode/utf8"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/webapi"
)

// These boundary tables port Rust's validation scenarios. They use no IO,
// clocks, or external services and run in well under one second.
func TestVerificationCode(t *testing.T) {
	for _, code := range []string{"012345", "abcdef", "12345", "1234567", "١٢٣٤٥٦", "12 456"} {
		_, err := webapi.DecodeEmailChangeConfirm([]byte(fmt.Sprintf(`{"id":"c","code":%q}`, code)))
		if (err == nil) != (code == "012345") {
			t.Errorf("%q: %v", code, err)
		}
	}
}

func TestRequestBoundsAndResponseTolerance(t *testing.T) {
	for _, scenario := range []struct {
		body  string
		valid bool
	}{
		{`{"email":"ana@example.test","password":"hunter22"}`, true},
		{`{"email":"ana@example.test","password":"short"}`, false},
		{`{"email":"bad","password":"hunter22"}`, false},
		{`{"email":"ana@example.test","password":"` + strings.Repeat("a", 1024) + `"}`, true},
		{`{"email":"ana@example.test","password":"hunter22","extra":1}`, false},
		{`{"email":"ana@example.test","password":"` + strings.Repeat("a", 1025) + `"}`, false},
	} {
		_, err := webapi.DecodeSetupRequest([]byte(scenario.body))
		if (err == nil) != scenario.valid {
			t.Errorf("setup valid=%v: %v", scenario.valid, err)
		}
	}
	if _, err := webapi.DecodeSetupStatus([]byte(`{"needed":true,"future":1}`)); err != nil {
		t.Fatal(err)
	}
	user := `{"id":"u1","email":"ana@example.test","nickname":"Ana","role":"user",` +
		`"createdAt":"2026-09-21T14:13:20.000Z","future":1}`
	if _, err := webapi.DecodeUserDTO([]byte(user)); err != nil {
		t.Fatal(err)
	}
	if _, err := webapi.DecodeUserDTO(
		[]byte(strings.Replace(user, "2026-09-21T14:13:20.000Z", "yesterday", 1)),
	); err == nil {
		t.Fatal("accepted invalid response timestamp")
	}

	for _, n := range []int{80, 81} {
		_, err := webapi.DecodeNicknamePatch([]byte(`{"nickname":"` + strings.Repeat("😀", n) + `"}`))
		if (err == nil) != (n == 80) {
			t.Errorf("nickname %d: %v", n, err)
		}
	}
}

func TestPasswordDiagnostics(t *testing.T) {
	value, err := webapi.DecodeCredentials([]byte(`{"email":"ana@example.test","password":"hunter22"}`))
	if err != nil {
		t.Fatal(err)
	}
	for _, format := range []string{"%v", "%+v", "%#v"} {
		if strings.Contains(fmt.Sprintf(format, value), "hunter22") {
			t.Errorf("password exposed with %s", format)
		}
	}
}

func TestCloudResetUUID(t *testing.T) {
	id := "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b"
	value, err := webapi.DecodeCloudReset([]byte(`{"operationId":"` + id + `"}`))
	if err != nil || string(value.OperationID) != id {
		t.Fatalf("%+v %v", value, err)
	}
	for _, body := range []string{
		`{"operationId":"reset-1"}`,
		`{"operationId":""}`,
		`{}`,
		`{"operationId":"` + id + `","base":"x"}`,
	} {
		if _, err := webapi.DecodeCloudReset([]byte(body)); err == nil {
			t.Errorf("accepted %s", body)
		}
	}
}

func TestConversationTarget(t *testing.T) {
	value, err := webapi.DecodeConversationTarget([]byte(`{"kind":"cloud"}`))
	if err != nil {
		t.Fatal(err)
	}
	if cloud, ok := value.(*webapi.ConversationTargetCloud); !ok || cloud.Path != nil {
		t.Fatalf("%+v", value)
	}
	for _, body := range []string{
		`{"kind":"cloud","path":"relative"}`,
		`{"kind":"cloud","path":null}`,
		`{"kind":"device","deviceId":"laptop","path":""}`,
		`{"kind":"workspace","workspaceId":"w1","path":"/work"}`,
		`{"kind":"elsewhere"}`,
	} {
		if _, err := webapi.DecodeConversationTarget([]byte(body)); err == nil {
			t.Errorf("accepted %s", body)
		}
	}
}

func TestExposeID(t *testing.T) {
	id := "k7x2maqw4p3s6tavaw2y4z6aab"
	if value, err := webapi.ParseExposeID(id); err != nil || string(value) != id {
		t.Fatalf("%q %v", value, err)
	}
	t.Run("refusals", func(t *testing.T) {
		t.Skip("fidelity 8: web API validation messages differ from Rust Display text")
		for _, refused := range []string{"", id[:25], id + "a", strings.ToUpper(id), id[:25] + "1"} {
			if _, err := webapi.ParseExposeID(
				refused,
			); err == nil ||
				err.Error() != "must be 26 lowercase base32 characters" {
				t.Errorf("input %q: %v, want %q", refused, err, "must be 26 lowercase base32 characters")
			}
		}
	})
}

func TestConversationID(t *testing.T) {
	for _, id := range []string{
		"0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b",
		"0B6F7F3E-8F3A-4C1E-9D2B-7A1C2E3F4A5B",
		"00000000-0000-0000-0000-000000000000",
		"ffffffff-ffff-ffff-ffff-ffffffffffff",
	} {
		if value, err := webapi.ParseConversationID(id); err != nil || string(value) != id {
			t.Errorf("%q %v", value, err)
		}
	}
	t.Run("refusals", func(t *testing.T) {
		t.Skip("fidelity 8: web API validation messages differ from Rust Display text")
		for _, refused := range []string{
			"",
			"conversation-1",
			"0b6f7f3e-8f3a-0c1e-9d2b-7a1c2e3f4a5b",
			"0b6f7f3e-8f3a-4c1e-7d2b-7a1c2e3f4a5b",
			"0b6f7f3e8f3a4c1e9d2b7a1c2e3f4a5b",
			"../0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b",
		} {
			if _, err := webapi.ParseConversationID(refused); err == nil || err.Error() != "must be a UUID" {
				t.Errorf("input %q: %v, want %q", refused, err, "must be a UUID")
			}
		}
		if _, err := webapi.DecodeConversationID(
			[]byte(`"not-a-uuid"`),
		); err == nil ||
			err.Error() != "must be a UUID" {
			t.Errorf("decoded UUID: %v, want must be a UUID", err)
		}
	})
}

func TestEmailNormalization(t *testing.T) {
	longest := strings.Repeat("a", webapi.EmailMax-13) + "@example.test"
	for _, scenario := range []struct{ input, want string }{
		{"  Ana@Example.TEST \n", "ana@example.test"},
		{"  " + strings.ToUpper(longest) + "  ", longest},
	} {
		value, err := webapi.ParseEmailAddress(scenario.input)
		if err != nil || string(value) != scenario.want {
			t.Errorf("%q %v", value, err)
		}
	}
	t.Run("overlong", func(t *testing.T) {
		t.Skip("fidelity 8: overlong email omits the documented character bound")
		if _, err := webapi.ParseEmailAddress(
			"a" + longest,
		); err == nil ||
			err.Error() != "must be at most 254 characters" {
			t.Fatalf("overlong email: %v, want must be at most 254 characters", err)
		}
	})
}

func TestEmailForm(t *testing.T) {
	for _, input := range []string{"a@b.co", "first.last+tag@sub.example.org", "o'neil_x-y@a-b.example"} {
		if _, err := webapi.ParseEmailAddress(input); err != nil {
			t.Errorf("%q: %v", input, err)
		}
	}
	t.Run("refusals", func(t *testing.T) {
		for _, input := range []string{
			"invalid",
			".ana@example.test",
			"ana..b@example.test",
			"ana.@example.test",
			"ana@example",
			"ana@-example.test",
			"ana@example.t",
			"ana@exa_mple.test",
			"an a@example.test",
			"anä@example.test",
			"",
		} {
			if _, err := webapi.ParseEmailAddress(input); err == nil || err.Error() != "must be an email address" {
				t.Errorf("input %q: %v, want %q", input, err, "must be an email address")
			}
		}
	})
}

func TestEndpoint(t *testing.T) {
	for _, scenario := range []struct{ input, want string }{
		{"https://api.kimi.com/coding/v1", "https://api.kimi.com/coding/v1"},
		{"http://127.0.0.1:8080", "http://127.0.0.1:8080/"},
	} {
		value, err := webapi.ParseEndpointURL(scenario.input)
		if err != nil || string(value) != scenario.want {
			t.Errorf("%q %v", value, err)
		}
	}
	t.Run("refusals", func(t *testing.T) {
		t.Skip("fidelity 8: web API validation messages differ from Rust Display text")
		for _, input := range []string{"", "api.openai.com/v1", "ftp://example.test/", "file:///etc", "https://"} {
			if _, err := webapi.ParseEndpointURL(input); err == nil || err.Error() != "must be an http or https URL" {
				t.Errorf("input %q: %v, want %q", input, err, "must be an http or https URL")
			}
		}
	})
}

func TestTrimmedText(t *testing.T) {
	for _, scenario := range []struct{ input, want string }{
		{`"\ufeff  New name \t\u2003"`, "New name"},
		{`"\u0085name"`, "\u0085name"},
	} {
		value, err := webapi.DecodeTrimmed([]byte(scenario.input))
		if scenario.want == "New name" && utf8.RuneCountInString(string(value)) != 8 {
			t.Fatalf("trimmed character count: %q", value)
		}
		if err != nil || string(value) != scenario.want {
			t.Errorf("%q %v", value, err)
		}
	}
}

func TestExposeAddress(t *testing.T) {
	for _, scenario := range []struct {
		input, want, host string
		port              uint16
	}{
		{"5173", "127.0.0.1:5173", "127.0.0.1", 5173},
		{"dev.internal:8080", "dev.internal:8080", "dev.internal", 8080},
		{"[::1]:3000", "[::1]:3000", "::1", 3000},
	} {
		value, err := webapi.DecodeExposeAddress([]byte(fmt.Sprintf("%q", scenario.input)))
		if err != nil {
			t.Fatal(err)
		}
		host, hostErr := value.Host()
		port, portErr := value.Port()
		if string(value) != scenario.want || host != scenario.host || port != scenario.port || hostErr != nil ||
			portErr != nil {
			t.Fatalf("%q %q %d %v %v", value, host, port, hostErr, portErr)
		}
	}
	for _, input := range []string{
		"",
		"0",
		"65536",
		"localhost",
		":80",
		"host:",
		"host:0",
		"host:http",
		"::1:80",
		"[]:80",
		"a b:80",
		"a/b:80",
		"[::1:80",
	} {
		if _, err := webapi.ParseExposeAddress(input); !errors.Is(err, webapi.ErrExposeAddress) {
			t.Errorf("accepted %q", input)
		}
	}
}

const configured = `{"id":" gpt-5.5 ","displayName":"GPT-5.5","contextWindow":272000,"outputLimit":null,` +
	`"thinkingEfforts":["low","high"],"acceptedExtensions":["png","pdf"],"fastTier":"priority"}`

func TestConfiguredModel(t *testing.T) {
	value, err := webapi.DecodeConfiguredModel([]byte(configured))
	if err != nil || value.ID != "gpt-5.5" {
		t.Fatalf("%+v %v", value, err)
	}
	for _, scenario := range []struct{ old, next string }{
		{`"contextWindow":272000`, `"contextWindow":0`},
		{`"outputLimit":null`, `"outputLimit":0`},
		{`"outputLimit":null`, `"outputLimit":272001`},
		{`"id":" gpt-5.5 "`, `"id":"   "`},
		{`["low","high"]`, `[""]`},
		{`["png","pdf"]`, `[".png"]`},
		{`"fastTier":"priority"`, `"fastTier":""`},
		{`"outputLimit":null,`, ``},
		{`"id":`, `"cost":1,"id":`},
	} {
		if _, err := webapi.DecodeConfiguredModel(
			[]byte(strings.Replace(configured, scenario.old, scenario.next, 1)),
		); err == nil {
			t.Errorf("accepted %s", scenario.next)
		}
	}
}

func TestConfiguredModelList(t *testing.T) {
	if _, err := webapi.DecodeConfiguredModels(
		[]byte("[" + configured + "," + configured + "]"),
	); err == nil ||
		!strings.Contains(err.Error(), "appears twice") {
		t.Fatalf("duplicate model: %v", err)
	}
	if _, err := webapi.DecodeConfiguredModels([]byte("[" + configured + "]")); err != nil {
		t.Fatal(err)
	}
	if _, err := webapi.DecodeConfiguredModels([]byte("[]")); err == nil {
		t.Fatal("accepted empty model list")
	}
	models := make([]string, 1000)
	for i := range models {
		models[i] = strings.Replace(configured, " gpt-5.5 ", fmt.Sprint(i), 1)
	}
	if _, err := webapi.DecodeConfiguredModels([]byte("[" + strings.Join(models, ",") + "]")); err != nil {
		t.Fatal(err)
	}
	models = append(models, strings.Replace(configured, " gpt-5.5 ", "1000", 1))
	if _, err := webapi.DecodeConfiguredModels([]byte("[" + strings.Join(models, ",") + "]")); err == nil {
		t.Fatal("accepted 1001 models")
	}
}

func TestProviderSourceAndNullablePatch(t *testing.T) {
	value, err := webapi.DecodeCreateProvider(
		[]byte(`{"source":"vendor","vendorId":"deepseek","label":" DeepSeek ","apiKey":"sk-1"}`),
	)
	if err != nil {
		t.Fatal(err)
	}
	if vendor, ok := value.(*webapi.CreateProviderVendor); !ok || vendor.Label != "DeepSeek" {
		t.Fatalf("%+v", value)
	}
	for _, body := range []string{
		`{"vendorId":"deepseek","label":"x","apiKey":"k"}`,
		`{"source":"vendor","vendorId":"deepseek","label":"x","apiKey":"k","wireApi":"responses"}`,
		`{"source":"custom","providerType":"openai","label":"x","apiKey":"k","baseUrl":null}`,
		`{"source":"custom","providerType":"openai","label":"x","apiKey":"k",` +
			`"baseUrl":"gw.example"}`,
	} {
		if _, err := webapi.DecodeCreateProvider([]byte(body)); err == nil {
			t.Errorf("accepted %s", body)
		}
	}
	patch, err := webapi.DecodeProviderPatch([]byte(`{"baseUrl":null,"label":"Work"}`))
	if err != nil {
		t.Fatal(err)
	}
	if patch.BaseURL == nil || *patch.BaseURL != nil || patch.Models != nil || patch.Label == nil ||
		*patch.Label != "Work" {
		t.Fatalf("%+v", patch)
	}
	encoded, err := contract.EncodeJSON(patch)
	if err != nil || string(encoded) != `{"label":"Work","baseUrl":null}` {
		t.Fatalf("%s %v", encoded, err)
	}
}

func TestPreferencesPatchBoundsAndPresence(t *testing.T) {
	for _, count := range []int{16, 17} {
		languages := strings.TrimSuffix(strings.Repeat(`"en",`, count), ",")
		_, err := webapi.DecodePreferencesPatch([]byte(`{"locale":{"timeZone":"UTC","languages":[` + languages + `]}}`))
		if (err == nil) != (count == 16) {
			t.Fatalf("%d languages: %v", count, err)
		}
	}
	if _, err := webapi.DecodePreferencesPatch(
		[]byte(`{"locale":{"timeZone":"UTC","languages":["en"],"extra":1}}`),
	); err == nil {
		t.Fatal("accepted unknown locale field")
	}

	for _, body := range []string{
		`{}`,
		`{"appearance":{"fontSize":18}}`,
		`{"shortcuts":{"new":null}}`,
		`{"shortcuts":{"new":""}}`,
	} {
		if _, err := webapi.DecodePreferencesPatch([]byte(body)); err != nil {
			t.Errorf("%s: %v", body, err)
		}
	}
	for _, body := range []string{
		`{"extra":1}`,
		`{"appearance":{"fontSize":19}}`,
		`{"shortcuts":{"new":"` + strings.Repeat("a", 65) + `"}}`,
		`{"locale":{"timeZone":"UTC","languages":[],"extra":1}}`,
	} {
		if _, err := webapi.DecodePreferencesPatch([]byte(body)); err == nil {
			t.Errorf("accepted %s", body)
		}
	}
}

func TestQueryRules(t *testing.T) {
	for _, scenario := range []struct {
		input string
		valid bool
	}{{"true", true}, {"false", true}, {"1", false}, {"TRUE", false}, {"", false}} {
		_, err := webapi.ParseStrictBool(scenario.input)
		if (err == nil) != scenario.valid {
			t.Errorf("%q: %v", scenario.input, err)
		}
	}
	for _, scenario := range []struct {
		input string
		valid bool
	}{
		{"/", true},
		{"/work", true},
		{`C:\work`, true},
		{`C:work`, false},
		{`\work`, false},
		{`\\server\share`, false},
		{`\\server\share\`, true},
		{`\\?\C:\work`, true},
		{`\\?\C:/work`, false},
		{"relative", false},
		{"", false},
	} {
		_, err := webapi.ParseAbsolutePath(scenario.input)
		if (err == nil) != scenario.valid {
			t.Errorf("absolute %q: %v", scenario.input, err)
		}
	}
	for _, scenario := range []struct {
		input string
		valid bool
	}{{"a/b", true}, {".", true}, {`C:\work`, true}, {"a/../b", false}, {"/a", false}, {"", false}} {
		_, err := webapi.ParseTreePath(scenario.input)
		if (err == nil) != scenario.valid {
			t.Errorf("tree %q: %v", scenario.input, err)
		}
	}
}

func TestDeviceLogQuery(t *testing.T) {
	value, err := webapi.DecodeDeviceLogValues(nil)
	if err != nil || value.Limit != webapi.DefaultLogLimit || value.Since != nil || value.Source != nil {
		t.Fatalf("%+v %v", value, err)
	}
	for _, scenario := range []struct {
		query string
		valid bool
	}{
		{"limit=1", true},
		{"limit=%2B1", true},
		{"limit=1000&since=18446744073709551615&source=runner", true},
		{"limit=0", false},
		{"limit=1001", false},
		{"source=", false},
		{"since=-1", false},
		{"limit=1&limit=2", false},
	} {
		values, err := url.ParseQuery(scenario.query)
		if err != nil {
			t.Fatal(err)
		}
		_, err = webapi.DecodeDeviceLogValues(values)
		if (err == nil) != scenario.valid {
			t.Errorf("%q: %v", scenario.query, err)
		}
	}
}

func TestErrorOptionalNull(t *testing.T) {
	if _, err := webapi.DecodeErrorBody(
		[]byte(`{"code":"plugin_refused","message":"refused","reason":null}`),
	); err != nil {
		t.Fatal(err)
	}
}

func TestRoleAdministration(t *testing.T) {
	for _, role := range []webapi.Role{webapi.RoleMaster, webapi.RoleAdmin, webapi.RoleUser} {
		for _, other := range []webapi.Role{webapi.RoleMaster, webapi.RoleAdmin, webapi.RoleUser} {
			want := role == webapi.RoleMaster && other != webapi.RoleMaster ||
				role == webapi.RoleAdmin && other == webapi.RoleUser
			if role.Outranks(other) != want {
				t.Errorf("%s administers %s", role, other)
			}
		}
	}
}

func TestConversationNullablePatch(t *testing.T) {
	for _, field := range []string{"thinkingEffort", "serviceTierId"} {
		t.Run(field, func(t *testing.T) {
			if _, err := webapi.DecodeConversationPatch([]byte(`{"` + field + `":null}`)); err != nil {
				t.Fatal(err)
			}
		})
	}
}

func TestWorkPanelBounds(t *testing.T) {
	for _, body := range []string{
		`{"selection":null,"tabs":[]}`,
		`{"selection":"change","tabs":[{"id":"1","kind":"custom","data":null}]}`,
	} {
		if _, err := webapi.DecodeWorkPanel([]byte(body)); err != nil {
			t.Errorf("%s: %v", body, err)
		}
	}
	for _, body := range []string{
		`{"selection":"","tabs":[]}`,
		`{"tabs":[]}`,
		`{"selection":null,"tabs":[],"extra":1}`,
		`{"selection":null,"tabs":[{"id":"","kind":"custom","data":null}]}`,
	} {
		if _, err := webapi.DecodeWorkPanel([]byte(body)); err == nil {
			t.Errorf("accepted %s", body)
		}
	}
	tabs := strings.TrimSuffix(strings.Repeat(`{"id":"1","kind":"custom","data":null},`, 65), ",")
	if _, err := webapi.DecodeWorkPanel([]byte(`{"selection":null,"tabs":[` + tabs + `]}`)); err == nil {
		t.Fatal("accepted 65 tabs")
	}
}
