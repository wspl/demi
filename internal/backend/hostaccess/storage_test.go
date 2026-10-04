package hostaccess

import (
	"context"
	"database/sql"
	"errors"
	"net/url"
	"strings"
	"testing"

	"github.com/google/go-cmp/cmp"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"mvdan.cc/sh/v3/interp"
	"mvdan.cc/sh/v3/syntax"
)

func TestCommandOutputStoredWithRetentionRecordAndFailureReason(t *testing.T) {
	s := newTestShard(t)
	record := s.conversation(t)
	keeper := &commandKeeper{shard: s, id: record.ID}
	output := host.WholeOutput{
		Records: []host.OutputRecord{
			{Stream: "stdout", Bytes: []byte("<whole> & output\n")},
			{Stream: "stderr", Bytes: []byte("warning\n")},
		},
		Missing: &host.Missing{Bytes: 4, Reason: "not read"},
	}
	if err := keeper.KeepOutput(t.Context(), "1", output); err != nil {
		t.Fatal(err)
	}
	read := func(command core.CommandID) *database.CommandOutput {
		t.Helper()
		var row database.CommandOutput
		var found bool
		exists, err := s.ConversationDB(record.ID).Read(t.Context(), func(ctx context.Context, tx *sql.Tx) error {
			var err error
			row, found, err = database.ReadCommandOutput(ctx, tx, command)
			return err
		})
		if err != nil || !exists || !found {
			t.Fatalf("output row: %v, exists=%t, %v", row, exists, err)
		}
		return &row
	}
	row := read("1")
	stored, ok := row.Output.(*database.OutputStored)
	if !ok {
		t.Fatalf("not stored: %#v", row.Output)
	}
	bytes, exists, err := s.blobs.Read(t.Context(), stored.Blob)
	if err != nil || !exists {
		t.Fatalf("blob: %t, %v", exists, err)
	}
	decoded, err := remotehost.DecodeOutput(bytes, stored.Missing)
	if err != nil {
		t.Fatal(err)
	}
	if diff := cmp.Diff(output, decoded); diff != "" {
		t.Fatal(diff)
	}
	invalid := host.WholeOutput{Records: []host.OutputRecord{{Stream: "invalid"}}}
	if err := keeper.KeepOutput(t.Context(), "2", invalid); err != nil {
		t.Fatal(err)
	}
	refused, ok := read("2").Output.(*database.OutputNotStored)
	if !ok || refused.Reason == "" {
		t.Fatal("encoding failure did not leave not-stored record")
	}
}

func TestUnavailableUploadsGrantNothing(t *testing.T) {
	s := newTestShard(t)
	record := s.conversation(t)
	for _, reference := range []string{"not-an-id", "00000000-0000-4000-8000-000000000001"} {
		content, _, err := ResolveUpload(t.Context(), s, record.ID, nil, reference, "notes.txt")
		if err != nil || len(content) != 1 {
			t.Fatalf("missing upload: %v, %v", content, err)
		}
		if _, ok := content[0].(*core.UserText); !ok {
			t.Fatalf("missing upload is not a message: %#v", content)
		}
	}
	blob, err := s.blobs.Put(t.Context(), []byte("private"))
	if err != nil {
		t.Fatal(err)
	}
	upload, err := s.control.CreateAttachment(t.Context(), s.owner, "text/plain", 7, blob, nil)
	if err != nil {
		t.Fatal(err)
	}
	owner := s.owner
	s.owner = "another-user"
	content, _, err := ResolveUpload(t.Context(), s, record.ID, nil, string(upload.ID), "notes.txt")
	s.owner = owner
	if err != nil || len(content) != 1 {
		t.Fatalf("foreign upload: %v, %v", content, err)
	}
	if _, ok := content[0].(*core.UserText); !ok {
		t.Fatal("foreign upload leaked media")
	}
}

// Cost: in-process shell parsing and expansion; handlers launch no processes.
func TestRemoteReferenceReadsExactPathWithoutShellInjection(t *testing.T) {
	device := database.DeviceRecord{ID: "00000000-0000-4000-8000-000000000001", Name: "my laptop & cloud"}
	words := func(script string) []string {
		t.Helper()
		parsed, err := syntax.NewParser().Parse(strings.NewReader(script), "reference")
		if err != nil {
			t.Fatal(err)
		}
		var calls [][]string
		shell, err := interp.New(interp.ExecHandlers(func(_ interp.ExecHandlerFunc) interp.ExecHandlerFunc {
			return func(_ context.Context, args []string) error {
				calls = append(calls, append([]string(nil), args...))
				return nil
			}
		}))
		if err != nil {
			t.Fatal(err)
		}
		if err := shell.Run(t.Context(), parsed); err != nil {
			t.Fatal(err)
		}
		if len(calls) != 1 {
			t.Fatalf("reference executed %d commands: %v", len(calls), calls)
		}
		return calls[0]
	}
	for _, path := range []string{
		"/plain.txt",
		"/a 'quote' & $(touch injected); `false`",
		"/line\nnext\tfile",
		"/control\x01é",
	} {
		block, err := remoteReference(device, path)
		if err != nil {
			t.Fatal(err)
		}
		reference, ok := block.(*core.UserReference)
		if !ok {
			t.Fatalf("reference shape: %T", block)
		}
		parsed, err := url.Parse(reference.Reference)
		if err != nil {
			t.Fatal(err)
		}
		if parsed.Path != path || parsed.Query().Get("host") != device.Name ||
			parsed.Query().Get("deviceId") != string(device.ID) {
			t.Fatalf("reference changed its file: %s", reference.Reference)
		}
		command := words(parsed.Query().Get("readCommand"))
		if len(command) != 6 {
			t.Fatalf("host shell arguments: %v", command)
		}
		if diff := cmp.Diff([]string{"demi", "host", "shell", "--host", string(device.ID)}, command[:5]); diff != "" {
			t.Fatal(diff)
		}
		if diff := cmp.Diff([]string{"cat", "--", path}, words(command[5])); diff != "" {
			t.Fatal(diff)
		}
	}
	_, err := remoteReference(device, "/nul\x00file")
	if !errors.Is(err, ErrPathUnquotable) {
		t.Fatalf("NUL path: %v", err)
	}
}
