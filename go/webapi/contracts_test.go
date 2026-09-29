package webapi

import (
	"fmt"
	"strings"
	"testing"
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

func TestConfiguredModelLimits(t *testing.T) {
	model := `{"id":" m1 ","displayName":" First ","contextWindow":1000,"outputLimit":500,"thinkingEfforts":[],"acceptedExtensions":null,"fastTier":null}`
	list, err := Decode[ConfiguredModels]([]byte("[" + model + "]"))
	if err != nil {
		t.Fatal(err)
	}
	if list.Models[0].ID != "m1" {
		t.Fatal("model ID was not trimmed")
	}
	for _, input := range []string{"[]", "[" + model + "," + model + "]", "[" + strings.Replace(model, `"outputLimit":500`, `"outputLimit":1001`, 1) + "]"} {
		if _, err := Decode[ConfiguredModels]([]byte(input)); err == nil {
			t.Fatal("accepted invalid configured models")
		}
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
