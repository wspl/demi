package wiregen

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"github.com/wspl/demi/go/internal/wire"
	"github.com/wspl/demi/go/internal/wiregen/featuretest"
	"github.com/wspl/demi/go/internal/wiregen/packtest"
	"strings"
	"testing"
)

// Cost: in-memory JSON and MessagePack scenarios, no timers or external services.
func TestForeignEmbeddingAndNormalizedValue(t *testing.T) {
	for _, packed := range []bool{false, true} {
		decode := func(raw string) (featuretest.Embedded, error) {
			if packed {
				return featuretest.DecodeEmbeddedMsgpack(packJSON(t, raw))
			}
			return featuretest.DecodeEmbeddedJSON([]byte(raw))
		}
		value, err := decode(`{"low":1,"high":3,"label":"  abc  "}`)
		if err != nil || value.Label != "abc" || value.Low != 1 || value.High != 3 {
			t.Fatalf("packed=%v: %+v %v", packed, value, err)
		}
		var encoded []byte
		if packed {
			encoded, err = value.MarshalMsgpack()
			if err == nil {
				encoded, err = wire.MPJSON(encoded)
			}
		} else {
			encoded, err = featuretest.EncodeEmbeddedJSON(value)
		}
		if err != nil || string(encoded) != `{"low":1,"high":3,"label":"abc"}` {
			t.Fatalf("packed=%v: %s %v", packed, encoded, err)
		}
		for raw, path := range map[string]string{
			`{"low":0,"high":3,"label":"ok"}`:           "low",
			`{"low":4,"high":3,"label":"ok"}`:           "low exceeds high",
			`{"low":1,"high":3,"label":"   "}`:          "label",
			`{"low":1,"high":3,"label":"abcd"}`:         "label",
			`{"low":1,"high":3,"label":"a\u0000"}`:      "label",
			`{"low":1,"high":3,"label":null}`:           "label",
			`{"high":3,"label":"ok"}`:                   "low",
			`{"low":"bad","high":3,"label":"ok"}`:       "low",
			`{"low":1,"high":3,"label":"ok","extra":1}`: "extra",
		} {
			if _, err := decode(raw); err == nil || !strings.Contains(err.Error(), path) {
				t.Errorf("packed=%v input=%s: %v", packed, raw, err)
			}
		}
		value.Low = 4
		if _, err := featuretest.EncodeEmbeddedJSON(value); err == nil {
			t.Fatal("encoding lost owner check")
		}
		value.Low = 1
		value.Label = " x "
		if _, err := featuretest.EncodeEmbeddedJSON(value); err == nil {
			t.Fatal("encoding lost custom check")
		}
	}
}

func TestStandaloneNormalizedValueKeepsDeclaredRules(t *testing.T) {
	value, err := featuretest.DecodeTrimmedJSON([]byte(`"  ok  "`))
	if err != nil || value != "ok" {
		t.Fatalf("%q %v", value, err)
	}
	if _, err := featuretest.DecodeTrimmedJSON([]byte(`" a\u0000 "`)); err == nil {
		t.Fatal("lost named nonul rule")
	}
	if _, err := featuretest.EncodeTrimmedJSON(" x "); err == nil {
		t.Fatal("lost custom validation")
	}
}

func TestForeignContainersKeepTheirShapeAndOwnerChecks(t *testing.T) {
	for _, packed := range []bool{false, true} {
		decode := func(raw string) (featuretest.Containers, error) {
			if packed {
				return featuretest.DecodeContainersMsgpack(packJSON(t, raw))
			}
			return featuretest.DecodeContainersJSON([]byte(raw))
		}
		raw := `{"map":{"z":{"low":1,"high":2}},"list":[{"low":2,"high":3}]}`
		value, err := decode(raw)
		if err != nil || value.Map["z"].Low != 1 || len(value.List) != 1 {
			t.Fatalf("packed=%v: %+v %v", packed, value, err)
		}
		var encoded []byte
		if packed {
			encoded, err = value.MarshalMsgpack()
			if err == nil {
				encoded, err = wire.MPJSON(encoded)
			}
		} else {
			encoded, err = featuretest.EncodeContainersJSON(value)
		}
		if err != nil || string(encoded) != raw {
			t.Fatalf("%s %v", encoded, err)
		}
		for input, path := range map[string]string{
			`{"map":[],"list":[]}`: "map",
			`{"map":{},"list":{}}`: "list",
			`{"map":{"z":{"low":0,"high":2}},"list":[{"low":1,"high":2}]}`: `map["z"].low`,
			`{"map":{},"list":[{"low":4,"high":2}]}`:                       "list[0]",
			`{"map":{},"list":[]}`:                                         "list",
		} {
			if _, err := decode(input); err == nil || !strings.Contains(err.Error(), path) {
				t.Errorf("packed=%v: %v; want %s", packed, err, path)
			}
		}
	}
}

func TestOpaqueRepresentationModel(t *testing.T) {
	pkg, err := Load("featuretest")
	if err != nil {
		t.Fatal(err)
	}
	email := pkg.OpaqueScalars["Email"]
	if email.Format != "email" || len(email.SchemaRules) != 1 || email.SchemaRules[0].Max.Value != 254 || !email.SchemaInline || string(email.Schema) != `{"type":"string","maxLength":254}` {
		t.Fatalf("email metadata: %+v", email)
	}
	percent := pkg.OpaqueScalars["Percent"]
	if percent.WireKind != KindFloat || percent.SchemaInline || len(percent.Schema) == 0 || percent.SchemaRules[0].Max.Value != 100 {
		t.Fatalf("percent metadata: %+v", percent)
	}
	rep := pkg.OpaqueScalars["Configured"].Representation
	if rep.Kind != KindSlice || rep.Elem.Name != "Usage" || rep.Rules[0].Max.Value != 1000 {
		t.Fatalf("representation: %+v", rep)
	}
	metadata, err := json.Marshal(map[string]any{
		"email":      map[string]any{"format": email.Format, "inline": email.SchemaInline, "schema": jsontext.Value(email.Schema), "bounds": email.SchemaRules},
		"percent":    map[string]any{"kind": percent.WireKind, "inline": percent.SchemaInline, "schema": jsontext.Value(percent.Schema), "bounds": percent.SchemaRules},
		"configured": map[string]any{"kind": rep.Kind, "element": rep.Elem.Name, "bounds": rep.Rules},
		"alias":      pkg.OpaqueScalars["EmailAlias"].SchemaRef,
	}, json.Deterministic(true), jsontext.WithIndent("  "))
	if err != nil {
		t.Fatal(err)
	}
	compare(t, "testdata/backend/opaque-model.json.golden", append(metadata, '\n'))
	if pkg.OpaqueScalars["EmailAlias"].SchemaRef != "Email" {
		t.Fatal("lost named schema")
	}
	if len(pkg.Structs["Embedded"].ForeignEmbeds) != 1 || pkg.Structs["Embedded"].Fields[0].Owner == nil {
		t.Fatal("lost foreign embedding model")
	}
	for _, field := range pkg.Structs["Containers"].Fields {
		if field.Type.Qualifier != "exporttest" || field.Type.Elem.Src != "exporttest.Interval" {
			t.Fatalf("lost container owner: %+v", field.Type)
		}
	}
}

func TestWebFeatureDeclarationRefusals(t *testing.T) {
	for _, body := range []string{
		"//demi:decode\ntype T string",
		"//demi:opaque string chars=3..1\ntype T struct{}",
		"//demi:opaque string format=a format=b\ntype T struct{}",
		"//demi:opaque boolean range=1..2\ntype T struct{}",
		"//demi:opaque\n//demi:jsonschema inline []\ntype T struct{}",
		"//demi:opaque\n//demi:jsonschema inline {\"type\":\"string\",\"type\":\"number\"}\ntype T struct{}",
		"//demi:opaque\n//demi:jsonschema ref A.B\ntype T struct{}",
		"//demi:opaque\n//demi:representation Missing\ntype T struct{}",
		"//demi:opaque string\n//demi:representation Values\ntype T struct{Values []string `json:\"values\"`}",
	} {
		if _, err := LoadSource(map[string][]byte{"p.go": []byte("package p\n" + body)}); err == nil {
			t.Errorf("accepted %s", body)
		}
	}
	owner, err := Load("exporttest")
	if err != nil {
		t.Fatal(err)
	}
	for _, body := range []string{
		"exporttest.Interval `json:\"bad\"`",
		"exporttest.Choice",
		"exporttest.Interval; Low int `json:\"low\"`",
	} {
		_, err := loadSource(map[string][]byte{"p.go": []byte("package p\nimport \"example/exporttest\"\n//demi:wire\ntype T struct{" + body + "}\n")}, func(string) (*Package, error) { return owner, nil })
		if err == nil {
			t.Errorf("accepted embedding %s", body)
		}
	}
}

// Cost: declaration reads and Go generation only; no compiled subprocess fixtures.
func TestWebDeclarationsUseOrdinaryGenerator(t *testing.T) {
	pkg, err := Load("../../webapi")
	if err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"AccountDTO", "CatalogModel", "WorkingTreeChanges"} {
		if s := pkg.Structs[name]; s == nil || len(s.ForeignEmbeds) != 1 {
			t.Fatalf("%s lacks its foreign owner", name)
		}
	}
	if _, err := pkg.GenerateGo(); err != nil {
		t.Fatal(err)
	}
	source := []byte("package probe\nimport (\"github.com/wspl/demi/go/agentproto\";\"github.com/wspl/demi/go/core\")\n//demi:wire\ntype T struct {Failures agentproto.Failures `json:\"failures\" check:\"each(func=core.Validate)\"`}\n")
	alias, err := loadSource(map[string][]byte{"probe.go": source}, foreignLoader("."))
	if err != nil {
		t.Fatal(err)
	}
	typ := alias.Structs["T"].Fields[0].Type
	if typ.Kind != KindMap || typ.Src != "agentproto.Failures" || typ.Elem.Src != "core.ProviderFailureFacts" {
		t.Fatalf("unexpected alias: %+v", typ)
	}
	if _, err := alias.GenerateGo(); err != nil {
		t.Fatal(err)
	}
}

// Cost: in-memory codecs; a JSON-only scalar owner can be used on the runner wire.
func TestForeignNormalizedScalarOnMessagePackWire(t *testing.T) {
	value, err := packtest.DecodeNormalizedMsgpack(packJSON(t, `{"label":"  ok  "}`))
	if err != nil || value.Label != "ok" {
		t.Fatalf("%+v %v", value, err)
	}
	encoded, err := value.MarshalMsgpack()
	if err != nil {
		t.Fatal(err)
	}
	raw, err := wire.MPJSON(encoded)
	if err != nil || string(raw) != `{"label":"ok"}` {
		t.Fatalf("%s %v", raw, err)
	}
	if _, err := packtest.DecodeNormalizedMsgpack(packJSON(t, `{"label":"a\u0000"}`)); err == nil {
		t.Fatal("foreign owner rules were lost")
	}
}
