package tools

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/google/go-cmp/cmp"
)

func TestSchemaDeclaresIntegerWindowsAndHandles(t *testing.T) {
	definitions := Definitions()
	// Schema is inspected here as output, not decoded as production input.
	var schema struct {
		AdditionalProperties bool     `json:"additionalProperties"`
		Required             []string `json:"required"`
		Properties           map[string]struct {
			Type        string `json:"type"`
			Minimum     uint64 `json:"minimum"`
			Maximum     uint64 `json:"maximum"`
			MinLength   uint64 `json:"minLength"`
			Description string `json:"description"`
		} `json:"properties"`
	}
	if err := json.Unmarshal(definitions[0].InputSchema, &schema); err != nil {
		t.Fatal(err)
	}
	if schema.AdditionalProperties || cmp.Diff([]string{"script", "timeoutMs"}, schema.Required) != "" {
		t.Fatalf("unexpected required properties: %+v", schema)
	}
	window := schema.Properties["timeoutMs"]
	if window.Type != "integer" || window.Minimum != 1 || window.Maximum != 600000 || schema.Properties["shellId"].Type != "integer" {
		t.Fatalf("incorrect numeric schema: %+v", schema.Properties)
	}
	description := schema.Properties["description"]
	if description.Type != "string" || description.Description != "Concise title for the concrete user-visible state or result to make visible or confirm. Do not describe waiting, pausing, tool mechanics, generic actions, object labels, steps, tool names, ids, internals, or reasons." {
		t.Fatal("changed description")
	}
	for _, definition := range definitions {
		if strings.Contains(string(definition.InputSchema), `"$schema"`) || strings.Contains(string(definition.InputSchema), `"title"`) {
			t.Fatal("tool schema contains title or dialect")
		}
	}
	if err := json.Unmarshal(definitions[2].InputSchema, &schema); err != nil {
		t.Fatal(err)
	}
	if schema.Properties["stdin"].MinLength != 1 {
		t.Fatal("stdin allows empty input")
	}
	// Although advertised as integers, the model may write numbered handles as strings.
	for _, input := range []string{`{"commandId":17}`, `{"commandId":"17"}`, `{"commandId":"+0017"}`} {
		got, err := decodeCommandInput([]byte(input))
		if err != nil || got.CommandID != 17 {
			t.Errorf("numbered handle %s: %+v, %v", input, got, err)
		}
	}
}

func TestRefusalNamesToolAndOffendingField(t *testing.T) {
	for _, tc := range []struct{ tool, input, want string }{
		{"shell_exec", `{"script":"true","timeoutMs":0}`, "timeoutMs: 0 is not a whole number of milliseconds from 1 to 600000"},
		{"shell_exec", `{"script":"true","timeoutMs":600001}`, "timeoutMs: 600001 is not a whole number of milliseconds from 1 to 600000"},
		{"yield", `{"durationMs":0}`, "durationMs: 0 is not a whole number of milliseconds from 1 to 600000"},
		{"yield", `{"durationMs":600001}`, "durationMs: 600001 is not a whole number of milliseconds from 1 to 600000"},
		{"shell_write", `{"commandId":7,"stdin":""}`, "stdin: must not be empty; use shell_status to poll"},
	} {
		t.Run(tc.tool+tc.input, func(t *testing.T) {
			var err error
			switch tc.tool {
			case "shell_exec":
				_, err = decodeShellExecInput([]byte(tc.input))
			case "yield":
				_, err = decodeYieldInput([]byte(tc.input))
			case "shell_write":
				_, err = decodeShellWriteInput([]byte(tc.input))
			}
			if err == nil {
				t.Fatal("invalid input accepted")
			}
			want := tc.tool + " input is invalid:\n" + tc.want
			if got := inputRefusal(tc.tool, err).Error(); got != want {
				t.Fatal(cmp.Diff(want, got))
			}
		})
	}

	cases := []struct{ input, field string }{
		{`{"script":"true","timeoutMs":1,"shellId":"main"}`, "shellId"},
		{`{"script":"true","timeoutMs":1,"shellId":null}`, "shellId"},
		{`{"script":"true","timeoutMs":1,"maxOutputBytes":10}`, "maxOutputBytes"},
		{`{"script":"true","timeoutMs":1.5}`, "timeoutMs"},
		{`{"script":"true","timeoutMs":1,"description":null}`, "description"},
		{`"not json"`, ""},
	}
	for _, tc := range cases {
		t.Run(tc.input, func(t *testing.T) {
			_, err := decodeShellExecInput([]byte(tc.input))
			if err == nil {
				t.Fatal("invalid input accepted")
			}
			text := inputRefusal("shell_exec", err).Error()
			if !strings.HasPrefix(text, "shell_exec input is invalid:\n") || !strings.Contains(text, tc.field) {
				t.Fatal(text)
			}
		})
	}
	got, err := decodeShellExecInput([]byte(`{"script":"ls","timeoutMs":600000,"description":"Files"}`))
	if err != nil || got.Script != "ls" || got.TimeoutMS != 600000 || got.ShellID != nil {
		t.Fatalf("valid input: %+v %v", got, err)
	}
	for _, input := range []string{`{"script":"ls","timeoutMs":1,"shellId":3}`, `{"script":"ls","timeoutMs":1,"shellId":"3"}`} {
		got, err := decodeShellExecInput([]byte(input))
		if err != nil || got.ShellID == nil || *got.ShellID != 3 {
			t.Errorf("valid optional numbered handle %s: %+v %v", input, got, err)
		}
	}
}
