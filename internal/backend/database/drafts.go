package database

import (
	"context"
	"database/sql"
	"fmt"
	"reflect"
	"strings"

	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// Draft returns the conversation's draft: the empty one at revision 0 before its first
// save.
func (c *ControlService) Draft(
	ctx context.Context,
	conversation webapiproto.ConversationID,
) (webapiproto.ConversationDraft, error) {
	return controlCall(
		ctx,
		c,
		func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (webapiproto.ConversationDraft, error) {
			d, found, err := readDraft(ctx, tx, conversation)
			if err != nil {
				return webapiproto.ConversationDraft{}, err
			}
			if !found {
				return webapiproto.EmptyConversationDraft(), nil
			}
			return d.present(), nil
		},
	)
}

// SaveDraft saves `text` and `files` as the draft of `owner`'s conversation. The
// save always takes effect; when `base` is not the current revision, the
// version it replaces is kept as the replaced one, unless that version
// is empty or the one saved. Each upload is presented with what its
// record holds.
func (c *ControlService) SaveDraft(
	ctx context.Context,
	conversation webapiproto.ConversationID,
	owner webapiproto.UserID,
	base uint64,
	text string,
	files []StagedFile,
) (webapiproto.ConversationDraft, error) {
	return controlCall(
		ctx,
		c,
		func(ctx context.Context, tx *sql.Tx, now types.Timestamp) (webapiproto.ConversationDraft, error) {
			if err := draftWritable(ctx, tx, conversation); err != nil {
				return webapiproto.ConversationDraft{}, err
			}
			presented, err := presentDraftFiles(ctx, tx, owner, files)
			if err != nil {
				return webapiproto.ConversationDraft{}, err
			}
			version := draftVersion{Text: text, Files: presented}
			document, err := encoded(version)
			if err != nil {
				return webapiproto.ConversationDraft{}, err
			}
			if len(document) > webapiproto.DraftBytesMax {
				return webapiproto.ConversationDraft{}, ErrDraftTooLarge
			}
			current, found, err := readDraft(ctx, tx, conversation)
			if err != nil {
				return webapiproto.ConversationDraft{}, err
			}
			next := storedDraft{revision: 1, version: version, written: 1}
			if found {
				next.revision = current.revision + 1
				next.written = next.revision
				next.replaced = current.replaced
				same := reflect.DeepEqual(current.version, version)
				if same {
					next.written = current.written
				}
				if base < current.written && !current.version.empty() && !same {
					next.replaced = &webapiproto.ReplacedDraft{
						Revision: current.revision,
						Text:     current.version.Text,
						Files:    current.version.Files,
					}
				}
			}
			if err := writeDraft(ctx, tx, conversation, next, now); err != nil {
				return webapiproto.ConversationDraft{}, err
			}
			return next.present(), nil
		},
	)
}

// ChangeReplacedDraft restores or dismisses the replaced version of `revision`. A restore
// exchanges it with the draft, whose version becomes the replaced one
// unless it is empty.
func (c *ControlService) ChangeReplacedDraft(
	ctx context.Context,
	conversation webapiproto.ConversationID,
	action webapiproto.ReplacedAction,
	revision uint64,
) (webapiproto.ConversationDraft, error) {
	return controlCall(
		ctx,
		c,
		func(ctx context.Context, tx *sql.Tx, now types.Timestamp) (webapiproto.ConversationDraft, error) {
			if err := draftWritable(ctx, tx, conversation); err != nil {
				return webapiproto.ConversationDraft{}, err
			}
			current, found, err := readDraft(ctx, tx, conversation)
			if err != nil {
				return webapiproto.ConversationDraft{}, err
			}
			if !found || current.replaced == nil || current.replaced.Revision != revision {
				return webapiproto.ConversationDraft{}, ErrDraftChanged
			}
			next := storedDraft{revision: current.revision + 1, written: current.written, version: current.version}
			switch action {
			case webapiproto.ReplacedActionRestore:
				next.written = next.revision
				next.version = draftVersion{Text: current.replaced.Text, Files: current.replaced.Files}
				if !current.version.empty() {
					next.replaced = &webapiproto.ReplacedDraft{
						Revision: current.revision,
						Text:     current.version.Text,
						Files:    current.version.Files,
					}
				}
			case webapiproto.ReplacedActionDismiss:
			}
			if err := writeDraft(ctx, tx, conversation, next, now); err != nil {
				return webapiproto.ConversationDraft{}, err
			}
			return next.present(), nil
		},
	)
}

// A draft's text and files, as its row stores the draft and the replaced
// version.
// +demi:root
type draftVersion struct {
	Text  string                  `json:"text"`
	Files []webapiproto.DraftFile `json:"files"`
}

type storedDraft struct {
	revision uint64
	version  draftVersion
	written  uint64
	replaced *webapiproto.ReplacedDraft
}

func (v draftVersion) empty() bool {
	return strings.TrimSpace(v.Text) == "" && len(v.Files) == 0
}

func (d storedDraft) present() webapiproto.ConversationDraft {
	return webapiproto.ConversationDraft{
		Revision: d.revision,
		Text:     d.version.Text,
		Files:    d.version.Files,
		Replaced: d.replaced,
	}
}

func draftRow(r *storedRow) storedDraft {
	d := storedDraft{
		revision: r.count("revision"),
		version:  storedJSON(r, "document", decodeDraftVersion),
		written:  r.count("written"),
	}
	if r.values["replaced_revision"] != nil && r.values["replaced"] != nil {
		v := storedJSON(r, "replaced", decodeDraftVersion)
		d.replaced = &webapiproto.ReplacedDraft{Revision: r.count("replaced_revision"), Text: v.Text, Files: v.Files}
	} else if r.values["replaced_revision"] != nil || r.values["replaced"] != nil {
		r.bad("replaced", fmt.Errorf("a replaced version and its revision are stored together or not at all"))
	}
	return d
}

func readDraft(ctx context.Context, tx *sql.Tx, id webapiproto.ConversationID) (storedDraft, bool, error) {
	return queryRecord(
		ctx,
		tx,
		"conversation_drafts",
		"SELECT * FROM conversation_drafts WHERE conversation_id = ?",
		draftRow,
		id,
	)
}

func draftWritable(ctx context.Context, tx *sql.Tx, id webapiproto.ConversationID) error {
	var archived bool
	if err := tx.QueryRowContext(ctx, "SELECT archived FROM conversations WHERE id = ?", id).
		Scan(&archived); err != nil {
		return err
	}
	if archived {
		return ErrArchived
	}
	return nil
}

func writeDraft(
	ctx context.Context,
	tx *sql.Tx,
	id webapiproto.ConversationID,
	d storedDraft,
	now types.Timestamp,
) error {
	document, err := encoded(d.version)
	if err != nil {
		return err
	}
	var revision *uint64
	var replaced *string
	if d.replaced != nil {
		revision = &d.replaced.Revision
		value, err := encoded(draftVersion{Text: d.replaced.Text, Files: d.replaced.Files})
		if err != nil {
			return err
		}
		replaced = &value
	}
	at, err := now.Millisecond()
	if err != nil {
		return err
	}
	return execSQL(
		ctx,
		tx,
		`INSERT INTO conversation_drafts (
    conversation_id,
    revision,
    document,
    written,
    replaced_revision,
    replaced,
    updated_at
)
VALUES (?,?,?,?,?,?,?)
ON CONFLICT (conversation_id) DO UPDATE
SET
    revision=excluded.revision,
    document=excluded.document,
    written=excluded.written,
    replaced_revision=excluded.replaced_revision,
    replaced=excluded.replaced,
    updated_at=excluded.updated_at`,
		id,
		d.revision,
		document,
		d.written,
		revision,
		replaced,
		at,
	)
}

func presentDraftFiles(
	ctx context.Context,
	tx *sql.Tx,
	owner webapiproto.UserID,
	files []StagedFile,
) ([]webapiproto.DraftFile, error) {
	presented := make([]webapiproto.DraftFile, 0, len(files))
	for _, file := range files {
		switch f := file.(type) {
		case *StagedUpload:
			upload, found, err := attachmentByID(ctx, tx, f.ID)
			if err != nil {
				return nil, err
			}
			if !found || upload.Owner != owner {
				return nil, &UploadNotFoundError{Upload: f.ID}
			}
			presented = append(
				presented,
				&webapiproto.DraftFileUpload{
					Ref:       f.ID,
					FileName:  f.FileName,
					MediaType: upload.MediaType,
					Sha256:    upload.SHA256,
					Snippet:   upload.Snippet,
				},
			)
		case *StagedRemote:
			presented = append(presented, &webapiproto.DraftFileRemoteFile{DeviceID: f.DeviceID, Path: f.Path})
		}
	}
	return presented, nil
}
