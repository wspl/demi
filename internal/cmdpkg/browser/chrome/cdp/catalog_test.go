package cdp

import (
	"encoding/json"
	"testing"
)

func TestPinnedProtocolRejectsUnknownMalformedAndExternalSchemas(t *testing.T) {
	for _, test := range []struct {
		method, kind, value string
		valid               bool
	}{
		{"Runtime.evaluate", "params", `{"expression":"1"}`, true},
		{"Runtime.evaluate", "params", `{}`, false},
		{"Runtime.evaluate", "params", `{"expression":"1","extra":true}`, false},
		{"Runtime.evaluate", "params", `{"expression":false}`, false},
		{"Runtime.noSuchMethod", "params", `{}`, false},
		{"Runtime.evaluate", "returns", `{"result":{"type":"number","value":1}}`, true},
		{"Runtime.evaluate", "returns", `{"result":{}}`, false},
		{
			"Target.attachedToTarget", "event",
			`{"sessionId":"child","targetInfo":{"targetId":"frame","type":"iframe","title":"",` +
				`"url":"about:blank","attached":true,"canAccessOpener":false},"waitingForDebugger":false}`, true,
		},
	} {
		t.Run(test.method+"/"+test.kind+"/"+test.value, func(t *testing.T) {
			err := validatePinned(test.method, test.kind, []byte(test.value))
			if (err == nil) != test.valid {
				t.Fatal(err)
			}
		})
	}
	if _, err := pageSchema(json.RawMessage(`{"$ref":"https://unreachable.invalid/schema"}`)); err == nil {
		t.Fatal("external page schema accepted")
	}
	schema, err := pageSchema(
		json.RawMessage(
			`{"type":"object","properties":{"q":{"type":"string"}},"required":["q"],` +
				`"additionalProperties":false}`,
		),
	)
	if err != nil {
		t.Fatal(err)
	}
	if err := schema.Validate(map[string]any{"q": "query"}); err != nil {
		t.Fatal(err)
	}
	if err := schema.Validate(map[string]any{"q": false}); err == nil {
		t.Fatal("wrong page argument accepted")
	}
}
