package serdetest

import (
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/go/runnerproto"
)

const boot = `{"backendUrl":"http://backend","deviceToken":"opaque"}`

func ptr[T any](v T) *T { return &v }

func TestAnOpenTypeIgnoresUnknownMembers(t *testing.T) {
	note, err := decode[Note]([]byte(`{"extra":{"x":[1]},"title":"a","reset":"r","later":3,"more":null}`))
	want := Note{Title: "a", Reset: ptr("r"), Later: ptr(3)}
	if err != nil || !reflect.DeepEqual(note, want) {
		t.Errorf("%+v, %v; want %+v", note, err, want)
	}
}

func TestARequiredMemberMayBeNullButNotAbsent(t *testing.T) {
	note, err := decode[Note]([]byte(`{"title":"a","reset":null}`))
	if err != nil || note.Reset != nil {
		t.Errorf("null: %+v, %v", note, err)
	}
	// The member is written as null, not left out.
	data, err := encode(Note{Title: "a"})
	if err != nil || string(data) != `{"title":"a","reset":null}` {
		t.Errorf("encoded %s, %v", data, err)
	}
	for line, want := range map[string]string{
		`{"title":"a"}`:  "reset: required",
		`{"reset":null}`: "title: required",
		`{"title":"a","reset":null,"later":null}`: "later: must not be null",
		`{"title":"a","reset":1}`:                 "reset:",
		`{"title":"","reset":null}`:               "title: must have at least 1 character",
	} {
		_, err := decode[Note]([]byte(line))
		if err == nil || !strings.Contains(err.Error(), want) {
			t.Errorf("%s: %v, want %q", line, err, want)
		}
	}
}

func TestAnAdjacentlyTaggedUnionIsAnObjectOfItsTagAndItsContent(t *testing.T) {
	for _, line := range []string{
		`{"command":{"op":"stop","params":{}}}`,
		`{"command":{"op":"resize","params":{"bytes":7}}}`,
		`{"command":{"op":"start","params":{"device":"d","boot":` + boot + `}}}`,
	} {
		holder, err := decode[Holder]([]byte(line))
		if err != nil {
			t.Errorf("%s: %v", line, err)
			continue
		}
		data, err := encode(holder)
		if err != nil || string(data) != line {
			t.Errorf("%s: encoded %s, %v", line, data, err)
		}
	}
	// The content's own members are its variant's, whatever their names: the
	// tag is not one of them.
	holder, err := decode[Holder]([]byte(`{"command":{"params":{"op":"x","bytes":1},"op":"resize"}}`))
	if err == nil {
		t.Errorf("a resize with a member op: %+v", holder)
	}
	stop, err := decode[Holder]([]byte(`{"command":{"params":{"op":"x"},"op":"stop"}}`))
	if err != nil || stop.Command != (Stop{}) {
		t.Errorf("a stop whose open content has a member op: %+v, %v", stop, err)
	}
}

func TestAnAdjacentUnionRefusesWhatItsShapeDoesNotAllow(t *testing.T) {
	for name, test := range map[string]struct{ line, want string }{
		"no params":            {`{"command":{"op":"stop"}}`, "command.params: required"},
		"no op":                {`{"command":{"params":{}}}`, "command.op: required"},
		"an unknown op":        {`{"command":{"op":"halt","params":{}}}`, "command.op: must be one of: stop, start, resize"},
		"a member beside them": {`{"command":{"op":"stop","params":{},"more":1}}`, "command.more"},
		"params of a kind":     {`{"command":{"op":"stop","params":[]}}`, "command.params"},
		"a strict variant":     {`{"command":{"op":"resize","params":{"bytes":1,"more":1}}}`, "command.params.more"},
		"a rule of a variant":  {`{"command":{"op":"resize","params":{"bytes":0}}}`, "command.params.bytes: must be at least 1"},
		"the union is null":    {`{"command":null}`, "command"},
	} {
		_, err := decode[Holder]([]byte(test.line))
		if err == nil || !strings.Contains(err.Error(), test.want) {
			t.Errorf("%s: %v, want %q", name, err, test.want)
		}
	}
	if _, err := encode(Holder{}); err == nil {
		t.Error("a holder without a command was encoded")
	}
}

func TestAFlattenedUnionSharesTheObjectOfItsStruct(t *testing.T) {
	line := `{"id":"7","op":"start","params":{"device":"d","boot":` + boot + `,"limit":3},"tag":"t"}`
	request, err := decode[Request]([]byte(line))
	if err != nil {
		t.Fatal(err)
	}
	start, ok := request.Call.(Start)
	if request.ID != "7" || request.Tag == nil || *request.Tag != "t" || !ok || start.Device != "d" || *start.Limit != 3 {
		t.Errorf("decoded %+v", request)
	}
	data, err := encode(request)
	if err != nil || string(data) != line {
		t.Errorf("encoded %s, %v", data, err)
	}
	// The members come in any order, and the unknown ones, at the top and in an
	// open variant's content, are ignored.
	request, err = decode[Request]([]byte(`{"trace":1,"params":{"more":true},"op":"stop","id":"8"}`))
	if err != nil || request.ID != "8" || request.Call != (Stop{}) || request.Tag != nil {
		t.Errorf("reordered: %+v, %v", request, err)
	}
	for name, test := range map[string]struct{ line, want string }{
		"no id":          {`{"op":"stop","params":{}}`, "id: required"},
		"an empty id":    {`{"id":"","op":"stop","params":{}}`, "id: must have at least 1 character"},
		"no op":          {`{"id":"1","params":{}}`, "op: required"},
		"no params":      {`{"id":"1","op":"stop"}`, "params: required"},
		"an unknown op":  {`{"id":"1","op":"halt","params":{}}`, "op: must be one of: stop, start, resize"},
		"a variant rule": {`{"id":"1","op":"resize","params":{"bytes":0}}`, "params.bytes: must be at least 1"},
		"a null tag":     {`{"id":"1","op":"stop","params":{},"tag":null}`, "tag: must not be null"},
	} {
		_, err := decode[Request]([]byte(test.line))
		if err == nil || !strings.Contains(err.Error(), test.want) {
			t.Errorf("%s: %v, want %q", name, err, test.want)
		}
	}
	if _, err := encode(Request{ID: "1"}); err == nil {
		t.Error("a request without a call was encoded")
	}
	if _, err := encode(Request{Call: Stop{}}); err == nil {
		t.Error("a request without an id was encoded")
	}
}

func TestTheWireStructOfAnotherPackageIsDecodedAndCheckedByThatPackage(t *testing.T) {
	line := `{"boots":[` + boot + `],"byId":{"a":` + boot + `},"raw":null}`
	fleet, err := decode[Fleet]([]byte(line))
	if err != nil || len(fleet.Boots) != 1 || fleet.Boots[0] != (runnerproto.ManagedBoot{BackendURL: "http://backend", DeviceToken: "opaque"}) {
		t.Fatalf("%+v, %v", fleet, err)
	}
	if data, err := encode(fleet); err != nil || string(data) != line {
		t.Errorf("encoded %s, %v", data, err)
	}
	const secret = "two secret words"
	for name, test := range map[string]struct{ line, want string }{
		"a structure":  {`{"boots":[{"backendUrl":"http://backend"}],"byId":{},"raw":1}`, "boots[0].deviceToken: required"},
		"a rule of it": {`{"boots":[{"backendUrl":"http://backend","deviceToken":"` + secret + `"}],"byId":{},"raw":1}`, "boots[0].deviceToken: is not a device token"},
		"a map's rule": {`{"boots":[],"byId":{"k":{"backendUrl":"ftp://x","deviceToken":"opaque"}},"raw":1}`, `byId["k"].backendUrl`},
	} {
		_, err := decode[Fleet]([]byte(test.line))
		if err == nil || !strings.Contains(err.Error(), test.want) || strings.Contains(err.Error(), "secret") {
			t.Errorf("%s: %v, want %q, and no secret", name, err, test.want)
		}
	}
	_, err = decode[Request]([]byte(`{"id":"1","op":"start","params":{"device":"d","boot":{"backendUrl":"http://backend","deviceToken":"` + secret + `"}}}`))
	if err == nil || !strings.Contains(err.Error(), "params.boot.deviceToken: is not a device token") || strings.Contains(err.Error(), "secret") {
		t.Errorf("a boot in a variant: %v", err)
	}
}
