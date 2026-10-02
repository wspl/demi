package main

import (
	"encoding/json"
	"go/ast"
	"go/parser"
	"go/token"
	"reflect"
	"testing"

	"github.com/wspl/demi/tools/contractgen/testdata/packedonly"
	"github.com/wspl/demi/tools/contractgen/testdata/reachability"
)

// Mixed roots must expose only their requested wire APIs while sharing validation.
// Local generated boundaries and source inspection; budget one second, no IO
// beyond fixture reads and no processes or timed waits.
func TestIndependentWireReachability(t *testing.T) {
	for _, tc := range []struct {
		value     any
		json, msg bool
	}{
		{&reachability.JSONOnly{}, true, false},
		{&reachability.Packed{}, false, true},
		{&reachability.Shared{}, true, true},
		{&reachability.Item{}, false, true},
		{&reachability.Empty{}, false, true},
		{&packedonly.Value{}, false, true},
	} {
		_, encJSON := tc.value.(json.Marshaler)
		_, decJSON := tc.value.(json.Unmarshaler)
		_, encMsg := tc.value.(interface{ MarshalMsgpack() ([]byte, error) })
		_, decMsg := tc.value.(interface{ UnmarshalMsgpack([]byte) error })
		if encJSON != tc.json || decJSON != tc.json || encMsg != tc.msg || decMsg != tc.msg {
			t.Errorf("%T: JSON=(%v,%v), MessagePack=(%v,%v); want (%v,%v)", tc.value, encJSON, decJSON, encMsg, decMsg, tc.json, tc.msg)
		}
		if _, ok := tc.value.(interface{ Validate() error }); !ok {
			t.Errorf("%T lost validation", tc.value)
		}
	}
	original := reachability.Packed{Shared: reachability.Shared{Text: "both"}, Choice: &reachability.Item{Number: 2}}
	data, err := original.MarshalMsgpack()
	if err != nil {
		t.Fatal(err)
	}
	got, err := reachability.DecodePackedMsgpack(data)
	if err != nil || !reflect.DeepEqual(got, original) {
		t.Fatalf("MessagePack: %+v %v", got, err)
	}
	if err := reachability.ValidateChoice(&reachability.Item{Number: 0}); err == nil {
		t.Fatal("MessagePack-only variant lost its bounds")
	}
	if err := reachability.ValidateStandalone(&reachability.Empty{}); err != nil {
		t.Fatal(err)
	}
	jsonValue, err := reachability.DecodeJSONOnly([]byte(`{"shared":{"text":"both"}}`))
	if err != nil || jsonValue.Shared != original.Shared {
		t.Fatalf("JSON: %+v %v", jsonValue, err)
	}

	// Interface method checks cannot see standalone decoder functions or union
	// holders, so check those exported declarations at the generated boundary.
	file, err := parser.ParseFile(token.NewFileSet(), "testdata/reachability/contract_gen.go", nil, 0)
	if err != nil {
		t.Fatal(err)
	}
	names := map[string]bool{}
	for _, declaration := range file.Decls {
		switch d := declaration.(type) {
		case *ast.FuncDecl:
			if d.Recv == nil {
				names[d.Name.Name] = true
			}
		case *ast.GenDecl:
			for _, spec := range d.Specs {
				if typ, ok := spec.(*ast.TypeSpec); ok {
					names[typ.Name.Name] = true
				}
			}
		}
	}
	for _, name := range []string{"DecodeJSONOnly", "DecodeShared", "DecodeSharedMsgpack", "DecodePackedMsgpack", "DecodeChoiceMsgpack", "DecodeStandaloneMsgpack"} {
		if !names[name] {
			t.Errorf("missing %s", name)
		}
	}
	for _, name := range []string{"DecodeJSONOnlyMsgpack", "DecodePacked", "DecodeChoice", "ChoiceJSON", "StandaloneJSON", "DecodeUnreached", "DecodeUnreachedMsgpack"} {
		if names[name] {
			t.Errorf("unexpected %s", name)
		}
	}
}
