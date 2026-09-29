package machinesproto_test

import (
	"bytes"
	"encoding/json/v2"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/go/machinesproto"
)

// corpus is the machine-manager socket's recorded corpus, which the Rust
// programs' tests read too (crates/machines-protocol/tests/fixtures).
const corpus = "../../crates/machines-protocol/tests/fixtures"

// lines returns the corpus lines of direction by name.
func lines(t *testing.T, direction string) map[string][]byte {
	t.Helper()
	entries, err := os.ReadDir(filepath.Join(corpus, direction))
	if err != nil || len(entries) == 0 {
		t.Fatalf("%s has no lines: %v", direction, err)
	}
	found := map[string][]byte{}
	for _, entry := range entries {
		data, err := os.ReadFile(filepath.Join(corpus, direction, entry.Name()))
		if err != nil {
			t.Fatal(err)
		}
		found[strings.TrimSuffix(entry.Name(), ".ndjson")] = data
	}
	return found
}

func body(line []byte) []byte {
	return bytes.TrimSuffix(line, []byte("\n"))
}

func TestEveryRequestDecodesAndEncodesToTheSameBytes(t *testing.T) {
	for name, line := range lines(t, "backend-to-manager") {
		request, err := machinesproto.DecodeRequest(body(line))
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		encoded, err := machinesproto.EncodeRequest(request)
		if err != nil || !bytes.Equal(encoded, line) {
			t.Errorf("%s: %q, %v; want %q", name, encoded, err, line)
		}
	}
}

func TestEveryResponseDecodesAndEncodesToTheSameBytes(t *testing.T) {
	for name, line := range lines(t, "manager-to-backend") {
		response, err := machinesproto.DecodeResponse(body(line))
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		encoded, err := machinesproto.EncodeResponse(response)
		if err != nil || !bytes.Equal(encoded, line) {
			t.Errorf("%s: %q, %v; want %q", name, encoded, err, line)
		}
	}
}

// result decodes the ok line name and returns its result.
func result(t *testing.T, name string) []byte {
	t.Helper()
	response, err := machinesproto.DecodeResponse(body(lines(t, "manager-to-backend")[name]))
	ok, isOK := response.(machinesproto.OK)
	if err != nil || !isOK {
		t.Fatalf("%s is not an ok reply: %v, %v", name, response, err)
	}
	return ok.Result
}

func TestOkRepliesCarryTheirOperationsResults(t *testing.T) {
	state, err := machinesproto.DecodeImageState(result(t, "ok.image_state"))
	base := strings.Repeat("b", 64)
	want := machinesproto.ImageState{
		Generation:  "7d2c9b1a-5e4f-4a3b-8c2d-1e0f9a8b7c6d",
		BaseVersion: machinesproto.BaseVersion(base),
		SystemBytes: 1 << 30,
		HomeBytes:   2 << 30,
	}
	if err != nil || state != want {
		t.Errorf("image state: %+v, %v; want %+v", state, err, want)
	}
	reset, err := machinesproto.DecodeImageState(result(t, "ok.image_state.reset"))
	if err != nil || reset.ResetID == nil || *reset.ResetID != "reset-4f1e" {
		t.Errorf("a reset generation: %+v, %v", reset, err)
	}
	// A device without a generation has a null result, which is not an error.
	none, err := machinesproto.DecodeImageStateResult(result(t, "ok.image_state.none"))
	if err != nil || none != nil {
		t.Errorf("no image state: %+v, %v", none, err)
	}
	if encoded, err := machinesproto.EncodeImageStateResult(nil); err != nil || string(encoded) != "null" {
		t.Errorf("no image state is encoded as %s, %v", encoded, err)
	}
	if some, err := machinesproto.DecodeImageStateResult(result(t, "ok.image_state")); err != nil || some == nil || *some != want {
		t.Errorf("an image state result: %+v, %v", some, err)
	}
	var running machinesproto.RuntimeState
	if err := json.Unmarshal(result(t, "ok.runtime_state"), &running); err != nil || running != machinesproto.Running {
		t.Errorf("runtime state: %q, %v", running, err)
	}
	if string(result(t, "ok.runtime_state.stopped")) != `"stopped"` {
		t.Errorf("a stopped runtime state")
	}
	if string(result(t, "ok.current_base_version")) != `"`+base+`"` {
		t.Errorf("the base version")
	}
}

func TestRequestsIgnoreUnknownKeysAndRefuseMalformedValues(t *testing.T) {
	device := `"deviceId":"dev-1"`
	boot := `"boot":{"backendUrl":"http://backend","deviceToken":"opaque"}`
	for _, line := range []string{
		`{"id":"1","op":"hibernate","params":{` + device + `},"trace":"x"}`,
		`{"id":"1","op":"hibernate","params":{` + device + `,"force":true}}`,
		`{"id":"1","op":"reconcile","params":{"all":true}}`,
		`{"id":"1","op":"wake","params":{` + device + `,` + boot + `}}`,
		// The pattern of a device id or base version is the operation's to check.
		`{"id":"1","op":"reset","params":{"deviceId":"../x","operationId":"o","baseVersion":"../b"}}`,
	} {
		if _, err := machinesproto.DecodeRequest([]byte(line)); err != nil {
			t.Errorf("%s: %v", line, err)
		}
	}
	for _, line := range []string{
		`not json`,
		`{"id":"1","op":"hibernate","params":{"deviceId":"dev-1"}} trailing`,
		`{"id":"1","op":"reboot","params":{}}`,
		`{"id":"1","op":"hibernate"}`,
		`{"op":"reconcile","params":{}}`,
		`{"id":"","op":"reconcile","params":{}}`,
		`{"id":"1","op":"hibernate","params":{"deviceId":""}}`,
		`{"id":"1","op":"hibernate","params":{"deviceId":7}}`,
		`{"id":"1","op":"wake","params":{` + device + `}}`,
		`{"id":"1","op":"wake","params":{` + device + `,"boot":{"backendUrl":"http://backend","deviceToken":"opaque","command":"x"}}}`,
		`{"id":"1","op":"wake","params":{` + device + `,"boot":{"backendUrl":"ftp://backend","deviceToken":"opaque"}}}`,
		`{"id":"1","op":"wake","params":{` + device + `,"boot":{"backendUrl":"http://backend","deviceToken":"two words"}}}`,
		`{"id":"1","op":"grow_volume","params":{` + device + `,"volume":"home","bytes":0}}`,
		`{"id":"1","op":"grow_volume","params":{` + device + `,"volume":"home","bytes":-1}}`,
		`{"id":"1","op":"grow_volume","params":{` + device + `,"volume":"home","bytes":1.5}}`,
		`{"id":"1","op":"grow_volume","params":{` + device + `,"volume":"swap","bytes":1}}`,
		`{"id":"1","op":"reset","params":{` + device + `,"operationId":"","baseVersion":"b"}}`,
		`{"id":"1","op":"reset","params":{` + device + `,"operationId":"o"}}`,
		"{\"id\":\"1\",\"op\":\"reconcile\",\"params\":{},\"x\":\"\xff\"}",
	} {
		if _, err := machinesproto.DecodeRequest([]byte(line)); err == nil {
			t.Errorf("%s decoded", line)
		}
	}
}

func TestRefusalsNameTheFieldAndNeverTheValue(t *testing.T) {
	_, err := machinesproto.DecodeRequest([]byte(`{"id":"1","op":"wake","params":{"deviceId":"d","boot":{"backendUrl":"http://backend","deviceToken":"two secret words"}}}`))
	want := "invalid: params.boot.deviceToken: is not a device token"
	if err == nil || err.Error() != want {
		t.Errorf("%v, want %q", err, want)
	}
	if strings.Contains(err.Error(), "secret") {
		t.Error("the refusal shows the token")
	}
}

func TestResponsesRefuseUnknownTypesAndMissingNames(t *testing.T) {
	for _, line := range []string{
		`{"type":"done","id":"1"}`,
		`{"type":"ok","result":null}`,
		`{"type":"error","id":"1"}`,
		`{"type":"death","deviceId":""}`,
	} {
		if _, err := machinesproto.DecodeResponse([]byte(line)); err == nil {
			t.Errorf("%s decoded", line)
		}
	}
}

func TestAnImageStateRequiresEveryKeyAndValidNames(t *testing.T) {
	valid := `{"generation":"g-1","baseVersion":"b_1","resetId":null,"systemBytes":1,"homeBytes":2}`
	state, err := machinesproto.DecodeImageState([]byte(valid))
	if err != nil || state.Bytes(machinesproto.Home) != 2 || state.Bytes(machinesproto.System) != 1 {
		t.Fatalf("a valid state: %+v, %v", state, err)
	}
	encoded, err := machinesproto.EncodeImageState(state)
	if err != nil || string(encoded) != valid {
		t.Errorf("encoded %s, %v; want %s", encoded, err, valid)
	}
	for _, invalid := range []string{
		`{"generation":"g-1","baseVersion":"b_1","systemBytes":1,"homeBytes":2}`,
		`{"generation":"g/1","baseVersion":"b_1","resetId":null,"systemBytes":1,"homeBytes":2}`,
		`{"generation":"g-1","baseVersion":"","resetId":null,"systemBytes":1,"homeBytes":2}`,
		`{"generation":"g-1","baseVersion":"b_1","resetId":null,"systemBytes":0,"homeBytes":2}`,
		`{"generation":"g-1","baseVersion":"b_1","resetId":null,"systemBytes":1,"homeBytes":2,"x":1}`,
	} {
		if _, err := machinesproto.DecodeImageState([]byte(invalid)); err == nil {
			t.Errorf("%s decoded", invalid)
		}
	}
}

func TestImageNamesAreSinglePathComponents(t *testing.T) {
	for _, name := range []string{"dev-1", "A_b-9", "0f6c3d4e-8a9b-4c1d-9e2f-3a4b5c6d7e8f"} {
		if _, err := machinesproto.ParseDeviceID(name); err != nil {
			t.Errorf("%q: %v", name, err)
		}
	}
	for _, name := range []string{"", ".", "..", "a/b", "a b", "dev.1", "é"} {
		_, err := machinesproto.ParseDeviceID(name)
		var invalid *machinesproto.IDError
		if !errors.As(err, &invalid) {
			t.Errorf("%q parsed", name)
		}
	}
}

func TestABootCredentialNeverAppearsInFormattedOutput(t *testing.T) {
	request, err := machinesproto.DecodeRequest(body(lines(t, "backend-to-manager")["wake"]))
	if err != nil {
		t.Fatal(err)
	}
	for _, format := range []string{"%v", "%+v", "%#v"} {
		text := fmt.Sprintf(format, request)
		if !strings.Contains(text, "backend.example.com") || strings.Contains(text, "dt_6b1f0c2e9a7d4c3b") {
			t.Errorf("%s: %s", format, text)
		}
	}
}
