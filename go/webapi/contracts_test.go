package webapi

import (
	"fmt"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/runnerproto"
)

// Boundary scenarios from web-api's Rust tests. No processes or network;
// expected cost is under one second once the shared contracts are available.
func TestAccountInputs(t *testing.T) {
	setup, err := Decode[SetupRequest]([]byte(`{"email":"  Ana@Example.TEST \n","password":"12345678"}`))
	if err != nil {
		t.Fatal(err)
	}
	if setup.Email.String() != "ana@example.test" {
		t.Fatal("email was not normalized")
	}
	if strings.Contains(fmt.Sprintf("%v %#v", setup.Password, setup.Password), "12345678") {
		t.Fatal("password formatting exposes its contents")
	}
	for _, input := range []string{
		`{"email":"ana@example.test","password":"short"}`,
		`{"email":"ana..b@example.test","password":"12345678"}`,
		`{"email":"ana@example.test","password":"12345678","extra":true}`,
	} {
		if _, err := Decode[SetupRequest]([]byte(input)); err == nil {
			t.Fatalf("accepted invalid account input: %s", input)
		}
	}
	nickname, err := Decode[NicknamePatch]([]byte(`{"nickname":"\ufeff  New name \t\u2003"}`))
	if err != nil {
		t.Fatal(err)
	}
	if nickname.Nickname != "New name" {
		t.Fatal("nickname was not trimmed before validation")
	}
	if _, err := Decode[NicknamePatch]([]byte(`{"nickname":"   "}`)); err == nil {
		t.Fatal("accepted an empty trimmed nickname")
	}
}

func TestPatchPresence(t *testing.T) {
	for _, input := range []string{`{}`, `{"new":null}`, `{"new":"Ctrl+N"}`} {
		patch, err := Decode[ShortcutsPatch]([]byte(input))
		if err != nil {
			t.Fatal(err)
		}
		encoded, err := Encode(patch)
		if err != nil {
			t.Fatal(err)
		}
		if string(encoded) != input {
			t.Fatalf("patch presence changed: %s -> %s", input, encoded)
		}
	}
	if _, err := Decode[ProviderPatch]([]byte(`{"label":null}`)); err == nil {
		t.Fatal("null is not an optional label")
	}
}

func TestCheckedIdentitiesAndExposeAddresses(t *testing.T) {
	spelling := "0B6F7F3E-8F3A-4C1E-9D2B-7A1C2E3F4A5B"
	id, err := ParseConversationID(spelling)
	if err != nil || id.String() != spelling {
		t.Fatalf("UUID case changed: %v", err)
	}
	if _, err := Decode[CloudReset]([]byte(`{"operationId":"not-a-uuid"}`)); err == nil {
		t.Fatal("reset operation accepted a non-UUID")
	}
	for _, input := range []string{"5173", "dev.internal:8080", "[::1]:3000"} {
		address, err := ParseExposeAddress(input)
		if err != nil {
			t.Fatal(err)
		}
		if address.Host() == "" || address.Port() == 0 {
			t.Fatal("address lost host or port")
		}
	}
	address, err := ParseExposeAddress("005173")
	if err != nil || address.String() != "127.0.0.1:5173" {
		t.Fatalf("bare port was not expanded: %v", err)
	}
	for _, input := range []string{"", "0", "65536", "localhost", ":80", "host:", "host:0", "host:http", "::1:80", "[]:80", "a b:80", "a/b:80", "[::1:80"} {
		if _, err := ParseExposeAddress(input); err == nil {
			t.Fatalf("accepted expose address %q", input)
		}
	}
}

// A configured model states its facts within their bounds, and a list holds
// one to a thousand of them with distinct ids (the Rust's
// a_configured_model_states_its_facts_within_their_bounds and
// a_model_list_holds_one_to_a_thousand_models_with_distinct_ids).
// Cost: in-memory decoding only.
func TestConfiguredModelsStateTheirFactsWithinBounds(t *testing.T) {
	configured := func(field, value string) string {
		members := map[string]string{"id": `" gpt-5.5 "`, "displayName": `"GPT-5.5"`, "contextWindow": "272000", "outputLimit": "null", "thinkingEfforts": `["low","high"]`, "acceptedExtensions": `["png","pdf"]`, "fastTier": `"priority"`}
		if field != "" {
			members[field] = value
		}
		var body strings.Builder
		for _, name := range []string{"id", "displayName", "contextWindow", "outputLimit", "thinkingEfforts", "acceptedExtensions", "fastTier", "cost"} {
			if member, ok := members[name]; ok && member != "absent" {
				if body.Len() > 0 {
					body.WriteString(",")
				}
				fmt.Fprintf(&body, "%q:%s", name, member)
			}
		}
		return "{" + body.String() + "}"
	}
	list, err := Decode[ConfiguredModels]([]byte("[" + configured("", "") + "]"))
	if err != nil || list.Models[0].ID != "gpt-5.5" {
		t.Fatalf("%+v %v", list, err)
	}
	for _, refused := range [][2]string{
		{"contextWindow", "0"},
		{"outputLimit", "0"},
		{"outputLimit", "272001"},
		{"id", `"   "`},
		{"thinkingEfforts", `[""]`},
		{"acceptedExtensions", `[".png"]`},
		{"fastTier", `""`},
		{"outputLimit", "absent"},
		{"cost", "1"},
	} {
		if _, err := Decode[ConfiguredModels]([]byte("[" + configured(refused[0], refused[1]) + "]")); err == nil {
			t.Errorf("accepted %s: %s", refused[0], refused[1])
		}
	}
	if _, err := Decode[ConfiguredModels]([]byte("[]")); err == nil {
		t.Error("accepted an empty model list")
	}
	twice := "[" + configured("", "") + "," + configured("", "") + "]"
	// Diagnostics name the path and the rule in G0's words, not the Rust's.
	if _, err := Decode[ConfiguredModels]([]byte(twice)); err == nil || !strings.Contains(err.Error(), "[1].id: must be distinct") {
		t.Errorf("a model listed twice: %v", err)
	}
}

// A verification code is six ASCII digits (the Rust's
// a_verification_code_is_six_ascii_digits).
// Cost: in-memory decoding only.
func TestVerificationCodeIsSixASCIIDigits(t *testing.T) {
	confirm := func(code string) error {
		_, err := Decode[EmailChangeConfirm]([]byte(`{"id":"c","code":"` + code + `"}`))
		return err
	}
	if err := confirm("012345"); err != nil {
		t.Fatal(err)
	}
	for _, refused := range []string{"abcdef", "12345", "1234567", "١٢٣٤٥٦", "12 456"} {
		if confirm(refused) == nil {
			t.Errorf("accepted %q", refused)
		}
	}
}

// A conversation's target refuses what its kind does not hold, such as a
// relative Cloud directory (the Rust's a_target_refuses_what_its_kind_does_not_hold).
// Cost: in-memory decoding only.
func TestTargetRefusesWhatItsKindDoesNotHold(t *testing.T) {
	cloud, err := DecodeConversationTargetJSON([]byte(`{"kind":"cloud"}`))
	if err != nil || !reflect.DeepEqual(cloud, ConversationTargetCloud{}) {
		t.Fatalf("%#v %v", cloud, err)
	}
	if _, err := DecodeConversationTargetJSON([]byte(`{"kind":"cloud","path":"/work"}`)); err != nil {
		t.Fatal(err)
	}
	for _, refused := range []string{
		`{"kind":"cloud","path":"relative"}`,
		`{"kind":"cloud","path":null}`,
		`{"kind":"device","deviceId":"laptop","path":""}`,
		`{"kind":"workspace","workspaceId":"w1","path":"/work"}`,
		`{"kind":"elsewhere"}`,
	} {
		if _, err := DecodeConversationTargetJSON([]byte(refused)); err == nil {
			t.Errorf("accepted %s", refused)
		}
	}
}

// A new entry names its source, and a patch tells null from absent (the
// Rust's a_new_entry_names_its_source_and_a_patch_tells_null_from_absent).
// Cost: in-memory decoding only.
func TestNewEntryNamesItsSourceAndPatchTellsNullFromAbsent(t *testing.T) {
	entry, err := DecodeCreateProviderJSON([]byte(`{"source":"vendor","vendorId":"deepseek","label":" DeepSeek ","apiKey":"sk-1"}`))
	vendor, ok := entry.(CreateProviderVendor)
	if err != nil || !ok || vendor.Label != "DeepSeek" {
		t.Fatalf("%#v %v", entry, err)
	}
	for _, refused := range []string{
		`{"vendorId":"deepseek","label":"x","apiKey":"k"}`,
		`{"source":"vendor","vendorId":"deepseek","label":"x","apiKey":"k","wireApi":"responses"}`,
		`{"source":"custom","providerType":"openai","label":"x","apiKey":"k","baseUrl":null}`,
		`{"source":"custom","providerType":"openai","label":"x","apiKey":"k","baseUrl":"gw.example"}`,
	} {
		if _, err := DecodeCreateProviderJSON([]byte(refused)); err == nil {
			t.Errorf("accepted %s", refused)
		}
	}
	patch, err := Decode[ProviderPatch]([]byte(`{"baseUrl":null,"label":"Work"}`))
	if err != nil || patch.BaseURL == nil || *patch.BaseURL != nil || patch.Models != nil || *patch.Label != "Work" {
		t.Fatalf("%#v %v", patch, err)
	}
}

// Transcript diagnostics use the owner's value codec through a map alias.
// Pure boundary decoding/encoding; expected cost is under one second.
func TestTranscriptFailureFacts(t *testing.T) {
	input := `{"blocks":[],"failures":{"error1":{"retryAt":null}},"subagents":[]}`
	transcript, err := Decode[Transcript]([]byte(input))
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := Encode(transcript)
	if err != nil {
		t.Fatal(err)
	}
	if string(encoded) != input {
		t.Fatalf("diagnostics changed: %s", encoded)
	}
	for _, bad := range []string{
		`{"blocks":[],"failures":{"":{"retryAt":null}},"subagents":[]}`,
		`{"blocks":[],"failures":{"error1":{}},"subagents":[]}`,
		`{"blocks":[],"failures":{"error1":{"retryAt":false}},"subagents":[]}`,
	} {
		if _, err := Decode[Transcript]([]byte(bad)); err == nil {
			t.Fatalf("accepted %s", bad)
		}
	}
}

// The conversation order travels as the sync channel sends it: each id is
// checked, and an empty order is an order.
// Cost: in-memory codecs only.
func TestConversationOrderEventTravels(t *testing.T) {
	id, err := ParseConversationID("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b")
	if err != nil {
		t.Fatal(err)
	}
	for _, ids := range [][]ConversationID{{}, {id}} {
		encoded, err := EncodeSyncEventJSON(SyncEventConversationOrder{Ids: ids})
		if err != nil {
			t.Fatal(err)
		}
		decoded, err := DecodeSyncEventJSON(encoded)
		order, ok := decoded.(SyncEventConversationOrder)
		if err != nil || !ok || len(order.Ids) != len(ids) {
			t.Fatalf("%s: %#v %v", encoded, decoded, err)
		}
	}
	if encoded, err := EncodeSyncEventJSON(SyncEventConversationOrder{Ids: []ConversationID{{}}}); err == nil {
		t.Fatalf("encoded a zero id: %s", encoded)
	}
	if _, err := DecodeSyncEventJSON([]byte(`{"type":"conversation_order","ids":["not-a-uuid"]}`)); err == nil || !strings.Contains(err.Error(), "ids[0]") {
		t.Fatalf("decoded a bad id: %v", err)
	}
}

// The records that embed another package's struct travel as the Rust writes
// them: the embedded struct's members beside the record's own, in declaration
// order, and back to the same value.
// Cost: in-memory codecs only.
func TestRecordsWithEmbeddedStructsTravel(t *testing.T) {
	updated, err := core.ParseTimestamp("2026-09-18T14:00:00.000Z")
	if err != nil {
		t.Fatal(err)
	}
	account := AccountDTO{AccountInfo: core.AccountInfo{ID: "acct-1", Label: "ana@example.test", UpdatedAt: &updated}}
	travels(t, account, `{"id":"acct-1","label":"ana@example.test","detail":null,"updatedAt":"2026-09-18T14:00:00.000Z","quota":null}`)
	model := core.ProviderModel{ID: "model", DisplayName: "Model", ServiceTiers: []core.ServiceTier{}}
	catalog := CatalogModel{ProviderModel: model, Selection: model.Selection("provider", nil, nil)}
	travels(t, catalog, `{"id":"model","displayName":"Model","description":null,"contextWindow":null,"outputLimit":null,"supportsTools":null,"supportsAttachments":null,"supportsVideo":null,"acceptedExtensions":null,"supportsReasoning":null,"supportedThinkingEfforts":null,"defaultThinkingEffort":null,"canDisableThinking":null,"serviceTiers":[],"defaultServiceTierId":null,"cost":null,"selection":{"providerId":"provider","model":{"id":"model","name":"Model","contextWindow":0,"inputLimit":null,"outputLimit":null,"thinking":[],"acceptedExtensions":null},"thinking":null,"serviceTierId":null},"unnamedEffort":null}`)
	changes := WorkingTreeChanges{Root: "/work", GitChanges: runnerproto.GitChanges{Repository: true, Files: []runnerproto.GitChange{}}}
	travels(t, changes, `{"root":"/work","repository":true,"head":null,"files":[],"truncated":false,"watched":false}`)
}

// travels encodes value, compares the bytes with want, and decodes them back.
func travels[T any](t *testing.T, value T, want string) {
	t.Helper()
	encoded, err := Encode(value)
	if err != nil || string(encoded) != want {
		t.Fatalf("%T encodes as %s (%v), want %s", value, encoded, err, want)
	}
	decoded, err := Decode[T](encoded)
	if err != nil || !reflect.DeepEqual(decoded, value) {
		t.Fatalf("%T decodes as %#v (%v)", value, decoded, err)
	}
}
