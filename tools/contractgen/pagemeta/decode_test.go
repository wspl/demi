package pagemeta_test

import (
	"strings"
	"testing"

	"github.com/wspl/demi/tools/contractgen/pagemeta"
)

// The command interface must fail closed on incomplete or incompatible output.
// Cost: small in-memory JSON documents; no processes or waits.
func TestDecodeMetadata(t *testing.T) {
	valid := `[{"id":"test","package":"@demicodes/plugin-test",` +
		`"schemas":[{"direction":"receive","value":{"type":"null"}}],` +
		`"constants":[{"name":"VALUE","description":"","value":null}]}]`
	if _, err := pagemeta.Decode([]byte(valid)); err != nil {
		t.Fatal(err)
	}
	for name, data := range map[string]string{
		"trailing":               valid + " []",
		"unknown":                strings.Replace(valid, `"id":"test"`, `"id":"test","future":true`, 1),
		"missing page field":     strings.Replace(valid, `"id":"test",`, "", 1),
		"missing schema field":   strings.Replace(valid, `"direction":"receive",`, "", 1),
		"missing constant field": strings.Replace(valid, `"description":"",`, "", 1),
		"invalid direction":      strings.Replace(valid, "receive", "both", 1),
		"null pages":             "null",
		"null element":           "[null]",
		"null list": strings.Replace(
			valid,
			`"constants":[{"name":"VALUE","description":"","value":null}]`,
			`"constants":null`,
			1,
		),
	} {
		t.Run(name, func(t *testing.T) {
			if _, err := pagemeta.Decode([]byte(data)); err == nil {
				t.Fatal("accepted invalid metadata")
			}
		})
	}
}
