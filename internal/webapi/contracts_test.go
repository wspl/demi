package webapi_test

import (
	"fmt"
	"net/url"
	"strings"
	"testing"

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
	for _, tc := range []struct {
		body  string
		valid bool
	}{
		{`{"email":"ana@example.test","password":"hunter22"}`, true},
		{`{"email":"ana@example.test","password":"short"}`, false},
		{`{"email":"ana@example.test","password":"hunter22","extra":1}`, false},
		{`{"email":"ana@example.test","password":"` + strings.Repeat("a", 1025) + `"}`, false},
	} {
		_, err := webapi.DecodeSetupRequest([]byte(tc.body))
		if (err == nil) != tc.valid {
			t.Errorf("setup valid=%v: %v", tc.valid, err)
		}
	}
	if _, err := webapi.DecodeSetupStatus([]byte(`{"needed":true,"future":1}`)); err != nil {
		t.Fatal(err)
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
	for _, body := range []string{`{"operationId":"reset-1"}`, `{"operationId":""}`, `{}`, `{"operationId":"` + id + `","base":"x"}`} {
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
	for _, body := range []string{`{"kind":"cloud","path":"relative"}`, `{"kind":"cloud","path":null}`, `{"kind":"device","deviceId":"laptop","path":""}`, `{"kind":"workspace","workspaceId":"w1","path":"/work"}`, `{"kind":"elsewhere"}`} {
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
	for _, refused := range []string{"", id[:25], id + "a", strings.ToUpper(id), id[:25] + "1"} {
		if _, err := webapi.ParseExposeID(refused); err == nil {
			t.Errorf("accepted %q", refused)
		}
	}
}

func TestConversationID(t *testing.T) {
	for _, id := range []string{"0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b", "0B6F7F3E-8F3A-4C1E-9D2B-7A1C2E3F4A5B", "00000000-0000-0000-0000-000000000000", "ffffffff-ffff-ffff-ffff-ffffffffffff"} {
		if value, err := webapi.ParseConversationID(id); err != nil || string(value) != id {
			t.Errorf("%q %v", value, err)
		}
	}
	for _, refused := range []string{"", "conversation-1", "0b6f7f3e-8f3a-0c1e-9d2b-7a1c2e3f4a5b", "0b6f7f3e-8f3a-4c1e-7d2b-7a1c2e3f4a5b", "0b6f7f3e8f3a4c1e9d2b7a1c2e3f4a5b", "../0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b"} {
		if _, err := webapi.ParseConversationID(refused); err == nil {
			t.Errorf("accepted %q", refused)
		}
	}
	if _, err := webapi.DecodeConversationID([]byte(`"not-a-uuid"`)); err == nil {
		t.Fatal("accepted invalid UUID")
	}
}

func TestEmailNormalization(t *testing.T) {
	longest := strings.Repeat("a", webapi.EmailMax-13) + "@example.test"
	for _, tc := range []struct{ input, want string }{{"  Ana@Example.TEST \n", "ana@example.test"}, {"  " + strings.ToUpper(longest) + "  ", longest}} {
		value, err := webapi.ParseEmailAddress(tc.input)
		if err != nil || string(value) != tc.want {
			t.Errorf("%q %v", value, err)
		}
	}
	if _, err := webapi.ParseEmailAddress("a" + longest); err == nil {
		t.Fatal("accepted overlong email")
	}
}

func TestEmailForm(t *testing.T) {
	for _, input := range []string{"a@b.co", "first.last+tag@sub.example.org", "o'neil_x-y@a-b.example"} {
		if _, err := webapi.ParseEmailAddress(input); err != nil {
			t.Errorf("%q: %v", input, err)
		}
	}
	for _, input := range []string{"invalid", ".ana@example.test", "ana..b@example.test", "ana.@example.test", "ana@example", "ana@-example.test", "ana@example.t", "ana@exa_mple.test", "an a@example.test", "anä@example.test", ""} {
		if _, err := webapi.ParseEmailAddress(input); err == nil {
			t.Errorf("accepted %q", input)
		}
	}
}

func TestEndpoint(t *testing.T) {
	for _, tc := range []struct{ input, want string }{{"https://api.kimi.com/coding/v1", "https://api.kimi.com/coding/v1"}, {"http://127.0.0.1:8080", "http://127.0.0.1:8080/"}} {
		value, err := webapi.ParseEndpointURL(tc.input)
		if err != nil || string(value) != tc.want {
			t.Errorf("%q %v", value, err)
		}
	}
	for _, input := range []string{"", "api.openai.com/v1", "ftp://example.test/", "file:///etc", "https://"} {
		if _, err := webapi.ParseEndpointURL(input); err == nil {
			t.Errorf("accepted %q", input)
		}
	}
}

func TestTrimmedText(t *testing.T) {
	for _, tc := range []struct{ input, want string }{{`"\ufeff  New name \t\u2003"`, "New name"}, {`"\u0085name"`, "\u0085name"}} {
		value, err := webapi.DecodeTrimmed([]byte(tc.input))
		if err != nil || string(value) != tc.want {
			t.Errorf("%q %v", value, err)
		}
	}
}

func TestExposeAddress(t *testing.T) {
	for _, tc := range []struct {
		input, want, host string
		port              uint16
	}{{"5173", "127.0.0.1:5173", "127.0.0.1", 5173}, {"dev.internal:8080", "dev.internal:8080", "dev.internal", 8080}, {"[::1]:3000", "[::1]:3000", "::1", 3000}} {
		value, err := webapi.DecodeExposeAddress([]byte(fmt.Sprintf("%q", tc.input)))
		if err != nil {
			t.Fatal(err)
		}
		host, hostErr := value.Host()
		port, portErr := value.Port()
		if string(value) != tc.want || host != tc.host || port != tc.port || hostErr != nil || portErr != nil {
			t.Fatalf("%q %q %d %v %v", value, host, port, hostErr, portErr)
		}
	}
	for _, input := range []string{"", "0", "65536", "localhost", ":80", "host:", "host:0", "host:http", "::1:80", "[]:80", "a b:80", "a/b:80", "[::1:80"} {
		if _, err := webapi.ParseExposeAddress(input); err == nil {
			t.Errorf("accepted %q", input)
		}
	}
}

const configured = `{"id":" gpt-5.5 ","displayName":"GPT-5.5","contextWindow":272000,"outputLimit":null,"thinkingEfforts":["low","high"],"acceptedExtensions":["png","pdf"],"fastTier":"priority"}`

func TestConfiguredModel(t *testing.T) {
	value, err := webapi.DecodeConfiguredModel([]byte(configured))
	if err != nil || value.ID != "gpt-5.5" {
		t.Fatalf("%+v %v", value, err)
	}
	for _, tc := range []struct{ old, next string }{
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
		if _, err := webapi.DecodeConfiguredModel([]byte(strings.Replace(configured, tc.old, tc.next, 1))); err == nil {
			t.Errorf("accepted %s", tc.next)
		}
	}
}

func TestConfiguredModelList(t *testing.T) {
	if _, err := webapi.DecodeConfiguredModels([]byte("[" + configured + "]")); err != nil {
		t.Fatal(err)
	}
	for _, body := range []string{"[]", "[" + configured + "," + configured + "]"} {
		if _, err := webapi.DecodeConfiguredModels([]byte(body)); err == nil {
			t.Errorf("accepted %s", body)
		}
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
	value, err := webapi.DecodeCreateProvider([]byte(`{"source":"vendor","vendorId":"deepseek","label":" DeepSeek ","apiKey":"sk-1"}`))
	if err != nil {
		t.Fatal(err)
	}
	if vendor, ok := value.(*webapi.CreateProviderVendor); !ok || vendor.Label != "DeepSeek" {
		t.Fatalf("%+v", value)
	}
	for _, body := range []string{`{"vendorId":"deepseek","label":"x","apiKey":"k"}`, `{"source":"vendor","vendorId":"deepseek","label":"x","apiKey":"k","wireApi":"responses"}`, `{"source":"custom","providerType":"openai","label":"x","apiKey":"k","baseUrl":null}`, `{"source":"custom","providerType":"openai","label":"x","apiKey":"k","baseUrl":"gw.example"}`} {
		if _, err := webapi.DecodeCreateProvider([]byte(body)); err == nil {
			t.Errorf("accepted %s", body)
		}
	}
	patch, err := webapi.DecodeProviderPatch([]byte(`{"baseUrl":null,"label":"Work"}`))
	if err != nil {
		t.Fatal(err)
	}
	if patch.BaseURL == nil || *patch.BaseURL != nil || patch.Models != nil || patch.Label == nil || *patch.Label != "Work" {
		t.Fatalf("%+v", patch)
	}
	encoded, err := contract.EncodeJSON(patch)
	if err != nil || string(encoded) != `{"label":"Work","baseUrl":null}` {
		t.Fatalf("%s %v", encoded, err)
	}
}

func TestPreferencesPatchBoundsAndPresence(t *testing.T) {
	for _, body := range []string{`{}`, `{"appearance":{"fontSize":18}}`, `{"shortcuts":{"new":null}}`, `{"shortcuts":{"new":""}}`} {
		if _, err := webapi.DecodePreferencesPatch([]byte(body)); err != nil {
			t.Errorf("%s: %v", body, err)
		}
	}
	for _, body := range []string{`{"extra":1}`, `{"appearance":{"fontSize":19}}`, `{"shortcuts":{"new":"` + strings.Repeat("a", 65) + `"}}`, `{"locale":{"timeZone":"UTC","languages":[],"extra":1}}`} {
		if _, err := webapi.DecodePreferencesPatch([]byte(body)); err == nil {
			t.Errorf("accepted %s", body)
		}
	}
}

func TestQueryRules(t *testing.T) {
	for _, tc := range []struct {
		input string
		valid bool
	}{{"true", true}, {"false", true}, {"1", false}, {"TRUE", false}, {"", false}} {
		_, err := webapi.ParseStrictBool(tc.input)
		if (err == nil) != tc.valid {
			t.Errorf("%q: %v", tc.input, err)
		}
	}
	for _, tc := range []struct {
		input string
		valid bool
	}{{"/", true}, {"/work", true}, {`C:\work`, true}, {`C:work`, false}, {`\work`, false}, {`\\server\share`, false}, {`\\server\share\`, true}, {`\\?\C:\work`, true}, {`\\?\C:/work`, false}, {"relative", false}, {"", false}} {
		_, err := webapi.ParseAbsolutePath(tc.input)
		if (err == nil) != tc.valid {
			t.Errorf("absolute %q: %v", tc.input, err)
		}
	}
	for _, tc := range []struct {
		input string
		valid bool
	}{{"a/b", true}, {".", true}, {`C:\work`, true}, {"a/../b", false}, {"/a", false}, {"", false}} {
		_, err := webapi.ParseTreePath(tc.input)
		if (err == nil) != tc.valid {
			t.Errorf("tree %q: %v", tc.input, err)
		}
	}
}

func TestDeviceLogQuery(t *testing.T) {
	value, err := webapi.DecodeDeviceLogValues(nil)
	if err != nil || value.Limit != webapi.DefaultLogLimit || value.Since != nil || value.Source != nil {
		t.Fatalf("%+v %v", value, err)
	}
	for _, tc := range []struct {
		query string
		valid bool
	}{{"limit=1", true}, {"limit=%2B1", true}, {"limit=1000&since=18446744073709551615&source=runner", true}, {"limit=0", false}, {"limit=1001", false}, {"source=", false}, {"since=-1", false}, {"limit=1&limit=2", false}} {
		values, err := url.ParseQuery(tc.query)
		if err != nil {
			t.Fatal(err)
		}
		_, err = webapi.DecodeDeviceLogValues(values)
		if (err == nil) != tc.valid {
			t.Errorf("%q: %v", tc.query, err)
		}
	}
}

func TestErrorOptionalNull(t *testing.T) {
	if _, err := webapi.DecodeErrorBody([]byte(`{"code":"plugin_refused","message":"refused","reason":null}`)); err != nil {
		t.Fatal(err)
	}
}

func TestRoleAdministration(t *testing.T) {
	for _, role := range []webapi.Role{webapi.RoleMaster, webapi.RoleAdmin, webapi.RoleUser} {
		for _, other := range []webapi.Role{webapi.RoleMaster, webapi.RoleAdmin, webapi.RoleUser} {
			want := role == webapi.RoleMaster && other != webapi.RoleMaster || role == webapi.RoleAdmin && other == webapi.RoleUser
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
	for _, body := range []string{`{"selection":null,"tabs":[]}`, `{"selection":"change","tabs":[{"id":"1","kind":"custom","data":null}]}`} {
		if _, err := webapi.DecodeWorkPanel([]byte(body)); err != nil {
			t.Errorf("%s: %v", body, err)
		}
	}
	for _, body := range []string{`{"selection":"","tabs":[]}`, `{"tabs":[]}`, `{"selection":null,"tabs":[],"extra":1}`, `{"selection":null,"tabs":[{"id":"","kind":"custom","data":null}]}`} {
		if _, err := webapi.DecodeWorkPanel([]byte(body)); err == nil {
			t.Errorf("accepted %s", body)
		}
	}
	tabs := strings.TrimSuffix(strings.Repeat(`{"id":"1","kind":"custom","data":null},`, 65), ",")
	if _, err := webapi.DecodeWorkPanel([]byte(`{"selection":null,"tabs":[` + tabs + `]}`)); err == nil {
		t.Fatal("accepted 65 tabs")
	}
}
