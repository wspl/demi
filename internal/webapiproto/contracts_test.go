package webapiproto_test

import (
	"errors"
	"fmt"
	"net/url"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/webapiproto"
)

// These boundary tables pin each contract's accepted and refused values. They use no IO,
// clocks, or external services and run in well under one second.
func TestVerificationCode(t *testing.T) {
	for _, code := range []string{"012345", "abcdef", "12345", "1234567", "١٢٣٤٥٦", "12 456"} {
		_, err := webapiproto.DecodeEmailChangeConfirm([]byte(fmt.Sprintf(`{"id":"c","code":%q}`, code)))
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
		_, err := webapiproto.DecodeSetupRequest([]byte(scenario.body))
		if (err == nil) != scenario.valid {
			t.Errorf("setup valid=%v: %v", scenario.valid, err)
		}
	}
	if _, err := webapiproto.DecodeSetupStatus([]byte(`{"needed":true,"future":1}`)); err != nil {
		t.Fatal(err)
	}
	user := `{"id":"u1","email":"ana@example.test","nickname":"Ana","role":"user",` +
		`"createdAt":"2026-09-21T14:13:20.000Z","future":1}`
	if _, err := webapiproto.DecodeUserDTO([]byte(user)); err != nil {
		t.Fatal(err)
	}
	if _, err := webapiproto.DecodeUserDTO(
		[]byte(strings.Replace(user, "2026-09-21T14:13:20.000Z", "yesterday", 1)),
	); err == nil {
		t.Fatal("accepted invalid response timestamp")
	}

	for _, n := range []int{80, 81} {
		_, err := webapiproto.DecodeNicknamePatch([]byte(`{"nickname":"` + strings.Repeat("😀", n) + `"}`))
		if (err == nil) != (n == 80) {
			t.Errorf("nickname %d: %v", n, err)
		}
	}
}

func TestPasswordDiagnostics(t *testing.T) {
	value, err := webapiproto.DecodeCredentials([]byte(`{"email":"ana@example.test","password":"hunter22"}`))
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
	value, err := webapiproto.DecodeCloudReset([]byte(`{"operationId":"` + id + `"}`))
	if err != nil || string(value.OperationID) != id {
		t.Fatalf("%+v %v", value, err)
	}
	for _, body := range []string{
		`{"operationId":"reset-1"}`,
		`{"operationId":""}`,
		`{}`,
		`{"operationId":"` + id + `","base":"x"}`,
	} {
		if _, err := webapiproto.DecodeCloudReset([]byte(body)); err == nil {
			t.Errorf("accepted %s", body)
		}
	}
}

func TestConversationTarget(t *testing.T) {
	value, err := webapiproto.DecodeConversationTarget([]byte(`{"kind":"cloud"}`))
	if err != nil {
		t.Fatal(err)
	}
	if cloud, ok := value.(*webapiproto.ConversationTargetCloud); !ok || cloud.Path != nil {
		t.Fatalf("%+v", value)
	}
	for _, body := range []string{
		`{"kind":"cloud","path":"relative"}`,
		`{"kind":"cloud","path":null}`,
		`{"kind":"device","deviceId":"laptop","path":""}`,
		`{"kind":"workspace","workspaceId":"w1","path":"/work"}`,
		`{"kind":"elsewhere"}`,
	} {
		if _, err := webapiproto.DecodeConversationTarget([]byte(body)); err == nil {
			t.Errorf("accepted %s", body)
		}
	}
}

func TestExposeID(t *testing.T) {
	id := "k7x2maqw4p3s6tavaw2y4z6aab"
	if value, err := webapiproto.ParseExposeID(id); err != nil || string(value) != id {
		t.Fatalf("%q %v", value, err)
	}
	for _, refused := range []string{"", id[:25], id + "a", strings.ToUpper(id), id[:25] + "1"} {
		if _, err := webapiproto.ParseExposeID(refused); err == nil {
			t.Errorf("accepted %q", refused)
		}
	}
}

func TestConversationID(t *testing.T) {
	for _, id := range []string{
		"0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b",
		"0B6F7F3E-8F3A-4C1E-9D2B-7A1C2E3F4A5B",
		"00000000-0000-0000-0000-000000000000",
		"ffffffff-ffff-ffff-ffff-ffffffffffff",
	} {
		if value, err := webapiproto.ParseConversationID(id); err != nil || string(value) != id {
			t.Errorf("%q %v", value, err)
		}
	}
	for _, refused := range []string{
		"",
		"conversation-1",
		"0b6f7f3e-8f3a-0c1e-9d2b-7a1c2e3f4a5b",
		"0b6f7f3e-8f3a-4c1e-7d2b-7a1c2e3f4a5b",
		"0b6f7f3e8f3a4c1e9d2b7a1c2e3f4a5b",
		"../0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b",
	} {
		if _, err := webapiproto.ParseConversationID(refused); err == nil {
			t.Errorf("accepted %q", refused)
		}
	}
	if _, err := webapiproto.DecodeConversationID([]byte(`"not-a-uuid"`)); err == nil {
		t.Error("decoded a conversation id that is not a UUID")
	}
}

func TestEmailNormalization(t *testing.T) {
	longest := strings.Repeat("a", webapiproto.EmailMax-13) + "@example.test"
	for _, scenario := range []struct{ input, want string }{
		{"  Ana@Example.TEST \n", "ana@example.test"},
		{"  " + strings.ToUpper(longest) + "  ", longest},
	} {
		value, err := webapiproto.ParseEmailAddress(scenario.input)
		if err != nil || string(value) != scenario.want {
			t.Errorf("%q %v", value, err)
		}
	}
	if _, err := webapiproto.ParseEmailAddress("a" + longest); err == nil {
		t.Error("accepted an email address longer than the bound")
	}
}

func TestEmailForm(t *testing.T) {
	for _, input := range []string{"a@b.co", "first.last+tag@sub.example.org", "o'neil_x-y@a-b.example"} {
		if _, err := webapiproto.ParseEmailAddress(input); err != nil {
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
			if _, err := webapiproto.ParseEmailAddress(input); err == nil || err.Error() != "must be an email address" {
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
		value, err := webapiproto.ParseEndpointURL(scenario.input)
		if err != nil || string(value) != scenario.want {
			t.Errorf("%q %v", value, err)
		}
	}
	for _, input := range []string{"", "api.openai.com/v1", "ftp://example.test/", "file:///etc", "https://"} {
		if _, err := webapiproto.ParseEndpointURL(input); err == nil {
			t.Errorf("accepted %q", input)
		}
	}
}

func TestTrimmedText(t *testing.T) {
	for _, scenario := range []struct{ input, want string }{
		{`"\ufeff  New name \t\u2003"`, "New name"},
		{`"\u0085name"`, "\u0085name"},
	} {
		value, err := webapiproto.DecodeTrimmed([]byte(scenario.input))
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
		value, err := webapiproto.DecodeExposeAddress([]byte(fmt.Sprintf("%q", scenario.input)))
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
		if _, err := webapiproto.ParseExposeAddress(input); !errors.Is(err, webapiproto.ErrExposeAddress) {
			t.Errorf("accepted %q", input)
		}
	}
}

const configured = `{"id":" gpt-5.5 ","displayName":"GPT-5.5","contextWindow":272000,"outputLimit":null,` +
	`"thinkingEfforts":["low","high"],"acceptedExtensions":["png","pdf"],"fastTier":"priority"}`

func TestConfiguredModel(t *testing.T) {
	value, err := webapiproto.DecodeConfiguredModel([]byte(configured))
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
		if _, err := webapiproto.DecodeConfiguredModel(
			[]byte(strings.Replace(configured, scenario.old, scenario.next, 1)),
		); err == nil {
			t.Errorf("accepted %s", scenario.next)
		}
	}
}

func TestConfiguredModelList(t *testing.T) {
	if _, err := webapiproto.DecodeConfiguredModels(
		[]byte("[" + configured + "," + configured + "]"),
	); err == nil ||
		!strings.Contains(err.Error(), "appears twice") {
		t.Fatalf("duplicate model: %v", err)
	}
	if _, err := webapiproto.DecodeConfiguredModels([]byte("[" + configured + "]")); err != nil {
		t.Fatal(err)
	}
	if _, err := webapiproto.DecodeConfiguredModels([]byte("[]")); err == nil {
		t.Fatal("accepted empty model list")
	}
	models := make([]string, 1000)
	for i := range models {
		models[i] = strings.Replace(configured, " gpt-5.5 ", fmt.Sprint(i), 1)
	}
	if _, err := webapiproto.DecodeConfiguredModels([]byte("[" + strings.Join(models, ",") + "]")); err != nil {
		t.Fatal(err)
	}
	models = append(models, strings.Replace(configured, " gpt-5.5 ", "1000", 1))
	if _, err := webapiproto.DecodeConfiguredModels([]byte("[" + strings.Join(models, ",") + "]")); err == nil {
		t.Fatal("accepted 1001 models")
	}
}

func TestProviderSourceAndNullablePatch(t *testing.T) {
	value, err := webapiproto.DecodeCreateProvider(
		[]byte(`{"source":"vendor","vendorId":"deepseek","label":" DeepSeek ","apiKey":"sk-1"}`),
	)
	if err != nil {
		t.Fatal(err)
	}
	if vendor, ok := value.(*webapiproto.CreateProviderVendor); !ok || vendor.Label != "DeepSeek" {
		t.Fatalf("%+v", value)
	}
	for _, body := range []string{
		`{"vendorId":"deepseek","label":"x","apiKey":"k"}`,
		`{"source":"vendor","vendorId":"deepseek","label":"x","apiKey":"k","wireApi":"responses"}`,
		`{"source":"custom","providerType":"openai","label":"x","apiKey":"k","baseUrl":null}`,
		`{"source":"custom","providerType":"openai","label":"x","apiKey":"k",` +
			`"baseUrl":"gw.example"}`,
	} {
		if _, err := webapiproto.DecodeCreateProvider([]byte(body)); err == nil {
			t.Errorf("accepted %s", body)
		}
	}
	patch, err := webapiproto.DecodeProviderPatch([]byte(`{"baseUrl":null,"label":"Work"}`))
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
		_, err := webapiproto.DecodePreferencesPatch(
			[]byte(`{"locale":{"timeZone":"UTC","languages":[` + languages + `]}}`),
		)
		if (err == nil) != (count == 16) {
			t.Fatalf("%d languages: %v", count, err)
		}
	}
	if _, err := webapiproto.DecodePreferencesPatch(
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
		if _, err := webapiproto.DecodePreferencesPatch([]byte(body)); err != nil {
			t.Errorf("%s: %v", body, err)
		}
	}
	for _, body := range []string{
		`{"extra":1}`,
		`{"appearance":{"fontSize":19}}`,
		`{"shortcuts":{"new":"` + strings.Repeat("a", 65) + `"}}`,
		`{"locale":{"timeZone":"UTC","languages":[],"extra":1}}`,
	} {
		if _, err := webapiproto.DecodePreferencesPatch([]byte(body)); err == nil {
			t.Errorf("accepted %s", body)
		}
	}
}

func TestQueryRules(t *testing.T) {
	for _, scenario := range []struct {
		input string
		valid bool
	}{{"true", true}, {"false", true}, {"1", false}, {"TRUE", false}, {"", false}} {
		_, err := webapiproto.ParseStrictBool(scenario.input)
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
		_, err := webapiproto.ParseAbsolutePath(scenario.input)
		if (err == nil) != scenario.valid {
			t.Errorf("absolute %q: %v", scenario.input, err)
		}
	}
	for _, scenario := range []struct {
		input string
		valid bool
	}{{"a/b", true}, {".", true}, {`C:\work`, true}, {"a/../b", false}, {"/a", false}, {"", false}} {
		_, err := webapiproto.ParseTreePath(scenario.input)
		if (err == nil) != scenario.valid {
			t.Errorf("tree %q: %v", scenario.input, err)
		}
	}
}

func TestDeviceLogQuery(t *testing.T) {
	value, err := webapiproto.DecodeDeviceLogValues(nil)
	if err != nil || value.Limit != webapiproto.DefaultLogLimit || value.Since != nil || value.Source != nil {
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
		_, err = webapiproto.DecodeDeviceLogValues(values)
		if (err == nil) != scenario.valid {
			t.Errorf("%q: %v", scenario.query, err)
		}
	}
}

func TestErrorOptionalNull(t *testing.T) {
	if _, err := webapiproto.DecodeErrorBody(
		[]byte(`{"code":"plugin_refused","message":"refused","reason":null}`),
	); err != nil {
		t.Fatal(err)
	}
}

func TestRoleAdministration(t *testing.T) {
	for _, role := range []webapiproto.Role{webapiproto.RoleMaster, webapiproto.RoleAdmin, webapiproto.RoleUser} {
		for _, other := range []webapiproto.Role{webapiproto.RoleMaster, webapiproto.RoleAdmin, webapiproto.RoleUser} {
			want := role == webapiproto.RoleMaster && other != webapiproto.RoleMaster ||
				role == webapiproto.RoleAdmin && other == webapiproto.RoleUser
			if role.Outranks(other) != want {
				t.Errorf("%s administers %s", role, other)
			}
		}
	}
}

func TestConversationNullablePatch(t *testing.T) {
	for _, field := range []string{"thinkingEffort", "serviceTierId"} {
		t.Run(field, func(t *testing.T) {
			if _, err := webapiproto.DecodeConversationPatch([]byte(`{"` + field + `":null}`)); err != nil {
				t.Fatal(err)
			}
		})
	}
}

// Panel request validation protects the HTTP boundary; the backend scenario
// covers mutation bounds. No IO, budget below one second.
func TestPanelRequestsValidateIDsAndObjectData(t *testing.T) {
	for _, body := range []string{
		`{"id":"a","kind":"page","data":{}}`,
		`{"id":"a","kind":"page","data":{},"index":null}`,
		`{"id":"a","kind":"page","data":{"nested":[null,1]},"index":64}`,
	} {
		if _, err := webapiproto.DecodeCreatePanelTab([]byte(body)); err != nil {
			t.Errorf("got %v; want valid %s", err, body)
		}
	}
	for _, body := range []string{
		`{"id":"","kind":"page","data":{}}`,
		`{"id":"a","kind":"page","data":null}`,
		`{"id":"a","kind":"page","data":[]}`,
		`{"id":"a","kind":"page","data":{},"index":65}`,
	} {
		if _, err := webapiproto.DecodeCreatePanelTab([]byte(body)); err == nil {
			t.Errorf("got valid; want refusal for %s", body)
		}
	}
}
