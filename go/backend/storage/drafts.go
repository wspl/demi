package storage

import (
	"context"
	"database/sql"
	"encoding/json/v2"
	"errors"
	"reflect"
	"strings"

	"github.com/wspl/demi/go/webapi"
)

type StagedFile interface{ stagedFile() }
type StagedUpload struct {
	ID       webapi.AttachmentID
	FileName string
}

func (StagedUpload) stagedFile() {}

type StagedRemote struct{ DeviceID, Path string }

func (StagedRemote) stagedFile() {}

type DraftRefusal struct {
	Kind   string
	Upload *webapi.AttachmentID
}

func (e *DraftRefusal) Error() string { return e.Kind }

//demi:wire
type DraftVersion struct {
	Text  string             `json:"text"`
	Files []webapi.DraftFile `json:"files" check:"each(func=webapi.ValidateDraftFile)"`
}

func (v DraftVersion) empty() bool { return strings.TrimSpace(v.Text) == "" && len(v.Files) == 0 }

type storedDraft struct {
	revision, written uint64
	version           DraftVersion
	replaced          *webapi.ReplacedDraft
}

func (d storedDraft) present() webapi.ConversationDraft {
	return webapi.ConversationDraft{Revision: d.revision, Text: d.version.Text, Files: d.version.Files, Replaced: d.replaced}
}
func readDraft(ctx context.Context, db database, id webapi.ConversationID) (*storedDraft, error) {
	var d storedDraft
	var document string
	var replacedRevision *uint64
	var replaced *string
	err := db.QueryRowContext(ctx, "SELECT revision,document,written,replaced_revision,replaced FROM conversation_drafts WHERE conversation_id=?", id.String()).Scan(storedCount{"conversation_drafts", "revision", &d.revision}, &document, storedCount{"conversation_drafts", "written", &d.written}, &replacedRevision, &replaced)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, sqliteError(err)
	}
	if err = json.Unmarshal([]byte(document), &d.version); err != nil {
		return nil, corrupt("conversation_drafts", "document", err)
	}
	if (replaced == nil) != (replacedRevision == nil) {
		return nil, &CorruptError{"conversation_drafts", "replaced", "a replaced version and its revision are stored together or not at all"}
	}
	if replaced != nil {
		var v DraftVersion
		if err = json.Unmarshal([]byte(*replaced), &v); err != nil {
			return nil, corrupt("conversation_drafts", "replaced", err)
		}
		d.replaced = &webapi.ReplacedDraft{Revision: *replacedRevision, Text: v.Text, Files: v.Files}
	}
	return &d, nil
}
func writeDraft(ctx context.Context, db database, id webapi.ConversationID, d storedDraft, now int64) error {
	document, err := json.Marshal(d.version)
	if err != nil {
		return err
	}
	var revision *uint64
	var replaced *string
	if d.replaced != nil {
		bytes, err := json.Marshal(DraftVersion{d.replaced.Text, d.replaced.Files})
		if err != nil {
			return err
		}
		text := string(bytes)
		replaced = &text
		revision = &d.replaced.Revision
	}
	_, err = db.ExecContext(ctx, `INSERT INTO conversation_drafts(conversation_id,revision,document,written,replaced_revision,replaced,updated_at) VALUES (?,?,?,?,?,?,?)
 ON CONFLICT(conversation_id) DO UPDATE SET revision=excluded.revision,document=excluded.document,written=excluded.written,replaced_revision=excluded.replaced_revision,replaced=excluded.replaced,updated_at=excluded.updated_at`, id.String(), d.revision, string(document), d.written, revision, replaced, now)
	return sqliteError(err)
}
func (c *Control) Draft(ctx context.Context, id webapi.ConversationID) (webapi.ConversationDraft, error) {
	d, err := readDraft(ctx, c.db, id)
	if err != nil {
		return webapi.ConversationDraft{}, err
	}
	if d == nil {
		return webapi.ConversationDraft{Files: []webapi.DraftFile{}}, nil
	}
	return d.present(), nil
}
func (c *Control) SaveDraft(ctx context.Context, id webapi.ConversationID, owner webapi.UserID, base uint64, text string, files []StagedFile) (webapi.ConversationDraft, error) {
	var empty webapi.ConversationDraft
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return empty, sqliteError(err)
	}
	defer tx.Rollback()
	var archived bool
	if err = tx.QueryRowContext(ctx, "SELECT archived FROM conversations WHERE id=?", id.String()).Scan(&archived); err != nil {
		return empty, sqliteError(err)
	}
	if archived {
		return empty, &DraftRefusal{Kind: "archived"}
	}
	presented := make([]webapi.DraftFile, 0, len(files))
	for _, file := range files {
		switch f := file.(type) {
		case StagedUpload:
			upload, err := attachment(ctx, tx, f.ID)
			if err != nil {
				return empty, err
			}
			if upload == nil || upload.Owner != owner {
				return empty, &DraftRefusal{Kind: "upload_not_found", Upload: &f.ID}
			}
			presented = append(presented, webapi.DraftFileUpload{Ref: f.ID, FileName: f.FileName, MediaType: upload.MediaType, SHA256: upload.SHA256, Snippet: upload.Snippet})
		case StagedRemote:
			presented = append(presented, webapi.DraftFileRemoteFile{DeviceID: f.DeviceID, Path: f.Path})
		default:
			return empty, errors.New("unknown staged file")
		}
	}
	version := DraftVersion{text, presented}
	encoded, err := json.Marshal(version)
	if err != nil {
		return empty, err
	}
	if len(encoded) > webapi.DraftBytesMax {
		return empty, &DraftRefusal{Kind: "too_large"}
	}
	current, err := readDraft(ctx, tx, id)
	if err != nil {
		return empty, err
	}
	next := storedDraft{revision: 1, written: 1, version: version}
	if current != nil {
		next.revision = current.revision + 1
		next.written = next.revision
		next.replaced = current.replaced
		equal := reflect.DeepEqual(current.version, version)
		if equal {
			next.written = current.written
		}
		if base < current.written && !current.version.empty() && !equal {
			next.replaced = &webapi.ReplacedDraft{Revision: current.revision, Text: current.version.Text, Files: current.version.Files}
		}
	}
	if err = writeDraft(ctx, tx, id, next, c.clock.Now().Millisecond()); err != nil {
		return empty, err
	}
	return next.present(), sqliteError(tx.Commit())
}
func (c *Control) ChangeReplacedDraft(ctx context.Context, id webapi.ConversationID, action webapi.ReplacedAction, revision uint64) (webapi.ConversationDraft, error) {
	var empty webapi.ConversationDraft
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return empty, sqliteError(err)
	}
	defer tx.Rollback()
	var archived bool
	if err = tx.QueryRowContext(ctx, "SELECT archived FROM conversations WHERE id=?", id.String()).Scan(&archived); err != nil {
		return empty, sqliteError(err)
	}
	if archived {
		return empty, &DraftRefusal{Kind: "archived"}
	}
	current, err := readDraft(ctx, tx, id)
	if err != nil {
		return empty, err
	}
	if current == nil || current.replaced == nil || current.replaced.Revision != revision {
		return empty, &DraftRefusal{Kind: "changed"}
	}
	next := storedDraft{revision: current.revision + 1, written: current.written, version: current.version}
	switch action {
	case webapi.ReplacedActionRestore:
		next.written = next.revision
		next.version = DraftVersion{current.replaced.Text, current.replaced.Files}
		if !current.version.empty() {
			next.replaced = &webapi.ReplacedDraft{Revision: current.revision, Text: current.version.Text, Files: current.version.Files}
		}
	case webapi.ReplacedActionDismiss:
	default:
		return empty, errors.New("unknown replaced draft action")
	}
	if err = writeDraft(ctx, tx, id, next, c.clock.Now().Millisecond()); err != nil {
		return empty, err
	}
	return next.present(), sqliteError(tx.Commit())
}
