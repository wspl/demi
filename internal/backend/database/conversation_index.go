package database

import (
	"context"
	"database/sql"
	"errors"
	"fmt"

	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// CreateConversation returns the owner's conversation of `id`, which is created when no
// conversation has the id: on the Cloud, with the placeholder title,
// first in the owner's sidebar. A retry of the owner's finds the one it
// created, in the spelling it was created with.
func (c *ControlService) CreateConversation(
	ctx context.Context,
	owner webapiproto.UserID,
	id webapiproto.ConversationID,
) (ConversationRecord, bool, error) {
	var created bool
	record, err := controlCall(
		ctx,
		c,
		func(ctx context.Context, tx *sql.Tx, now types.Timestamp) (ConversationRecord, error) {
			var reserved bool
			if err := tx.QueryRowContext(ctx, "SELECT EXISTS (SELECT 1 FROM conversation_fork_operations WHERE id = ?)", id).
				Scan(&reserved); err != nil {
				return ConversationRecord{}, err
			}
			if reserved {
				return ConversationRecord{}, ErrIDUnavailable
			}
			count, err := InsertConversation(
				ctx,
				tx,
				NewConversation{
					ID:     id,
					Owner:  owner,
					Title:  "New conversation",
					Origin: TitlePlaceholder,
					Target: &webapiproto.ConversationTargetCloud{},
					At:     now,
				},
			)
			if err != nil {
				return ConversationRecord{}, err
			}
			r, found, err := ConversationByID(ctx, tx, id)
			if err != nil {
				return ConversationRecord{}, err
			}
			if !found {
				return ConversationRecord{}, CorruptValue(
					"conversations",
					"id",
					errors.New("a conversation just created or found is missing"),
				)
			}
			if r.Owner != owner {
				return ConversationRecord{}, ErrIDUnavailable
			}
			created = count == 1
			return r, nil
		},
	)
	if err != nil {
		return ConversationRecord{}, false, err
	}
	return record, created, nil
}

// Conversation returns the conversation of `id`, in whichever case it is spelled.
func (c *ControlService) Conversation(
	ctx context.Context,
	id webapiproto.ConversationID,
) (ConversationRecord, bool, error) {
	var found bool
	record, err := controlCall(
		ctx,
		c,
		func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (ConversationRecord, error) {
			r, ok, err := ConversationByID(ctx, tx, id)
			found = ok
			return r, err
		},
	)
	return record, found && err == nil, err
}

// LastSwitch returns the conversation's latest target switch, which every node's next
// context block describes; none before its first.
func (c *ControlService) LastSwitch(ctx context.Context, id webapiproto.ConversationID) (*TargetSwitch, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (*TargetSwitch, error) {
		r, found, err := queryRecord(
			ctx,
			tx,
			"conversations",
			"SELECT last_switch FROM conversations WHERE id = ?",
			func(r *storedRow) *TargetSwitch {
				return optionalJSON(r, "last_switch", DecodeTargetSwitch)
			},
			id,
		)
		if !found {
			return nil, err
		}
		return r, err
	})
}

// Conversations returns the owner's conversations that are archived, or that are not, in
// sidebar order.
func (c *ControlService) Conversations(
	ctx context.Context,
	owner webapiproto.UserID,
	archived bool,
) ([]ConversationRecord, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) ([]ConversationRecord, error) {
		return queryRecords(
			ctx,
			tx,
			"conversations",
			"SELECT "+conversationColumns+`
FROM conversations
WHERE user_id = ? AND archived = ?
ORDER BY pinned DESC,sort_order,id`,
			conversationRow,
			owner,
			archived,
		)
	})
}

// ConversationOrder returns the ids of the owner's conversations in the order the product state
// lists them: the active ones in sidebar order, then the archived ones.
func (c *ControlService) ConversationOrder(
	ctx context.Context,
	owner webapiproto.UserID,
) ([]webapiproto.ConversationID, error) {
	return controlCall(
		ctx,
		c,
		func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) ([]webapiproto.ConversationID, error) {
			return queryRecords(
				ctx,
				tx,
				"conversations",
				"SELECT id FROM conversations WHERE user_id = ? ORDER BY archived,pinned DESC,sort_order,id",
				func(r *storedRow) webapiproto.ConversationID {
					return checked(r, "id", webapiproto.ParseConversationID)
				},
				owner,
			)
		},
	)
}

// MarkConversationRead acknowledges the output up to `revision`; an acknowledgement never
// moves the read revision back.
func (c *ControlService) MarkConversationRead(
	ctx context.Context,
	id webapiproto.ConversationID,
	revision uint64,
) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) error {
		return execSQL(
			ctx,
			tx,
			"UPDATE conversations SET read_revision = MAX(read_revision,?) WHERE id = ?",
			integer(revision),
			id,
		)
	})
}

// ChangeConversation applies `change` in one transaction. An archived conversation takes
// nothing but its restore; a rename that repeats the current title
// changes nothing, so the title keeps its origin.
func (c *ControlService) ChangeConversation(
	ctx context.Context,
	id webapiproto.ConversationID,
	change RecordChange,
) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, now types.Timestamp) error {
		var archived bool
		err := tx.QueryRowContext(ctx, "SELECT archived FROM conversations WHERE id = ?", id).Scan(&archived)
		if errors.Is(err, sql.ErrNoRows) {
			return ErrConversationNotFound
		}
		if err != nil {
			return err
		}
		if _, restoring := change.(*RecordArchived); archived && !restoring {
			return ErrArchived
		}
		switch ch := change.(type) {
		case *RecordArchived:
			err = execSQL(ctx, tx, "UPDATE conversations SET archived = ? WHERE id = ?", ch.Archived, id)
		case *RecordTitle:
			err = execSQL(
				ctx,
				tx,
				"UPDATE conversations SET title = ?2,title_origin = 'user' WHERE id = ?1 AND title <> ?2",
				id,
				ch.Title,
			)
		case *RecordPinned:
			err = execSQL(ctx, tx, "UPDATE conversations SET pinned = ? WHERE id = ?", ch.Pinned, id)
		case *RecordModel:
			var text string
			text, err = encoded(ch.Model)
			if err == nil {
				err = execSQL(ctx, tx, "UPDATE conversations SET model = ? WHERE id = ?", text, id)
			}
		case *RecordAttach:
			var inserted bool
			inserted, err = InsertAttachedHost(ctx, tx, id, ch.Host, now)
			if err == nil && inserted {
				err = advanceContext(ctx, tx, id)
			}
		case *RecordRename:
			return renameAttachedHost(ctx, tx, id, ch)
		case *RecordDetach:
			var removed bool
			removed, err = affected(
				ctx,
				tx,
				"DELETE FROM conversation_hosts WHERE conversation_id = ? AND device_id = ?",
				id,
				ch.Device,
			)
			if err == nil && removed {
				err = advanceContext(ctx, tx, id)
			}
		}
		return err
	})
}

// CountUserMessage counts one more message the user sent, which makes a generated title
// older than the conversation; answers how many there are now.
func (c *ControlService) CountUserMessage(ctx context.Context, id webapiproto.ConversationID) (uint64, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (uint64, error) {
		var count uint64
		err := tx.QueryRowContext(ctx, `UPDATE conversations
SET user_messages = user_messages + 1
WHERE id = ?
RETURNING user_messages`, id).
			Scan(&count)
		return count, err
	})
}

// TitleFromFirstMessage makes `title`, from the first message, the conversation's while its
// title is still the placeholder; answers whether it did, which makes
// this send the one a generated title may follow.
func (c *ControlService) TitleFromFirstMessage(
	ctx context.Context,
	id webapiproto.ConversationID,
	title string,
) (bool, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (bool, error) {
		return affected(
			ctx,
			tx,
			"UPDATE conversations SET title = ?,title_origin = 'message' WHERE id = ? AND title_origin = 'placeholder'",
			title,
			id,
		)
	})
}

// GeneratedTitle writes the generated `title` while the title is still `from`, the one
// its request began from, in the statement that checks it, so a rename
// that landed meanwhile stays. Either way the title is current for the
// `seen` messages the request read. Answers whether it was written.
func (c *ControlService) GeneratedTitle(
	ctx context.Context,
	id webapiproto.ConversationID,
	title string,
	from string,
	seen uint64,
) (bool, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (bool, error) {
		won, err := affected(
			ctx,
			tx,
			"UPDATE conversations SET title = ?,title_origin = 'generated' WHERE id = ? AND title = ?",
			title,
			id,
			from,
		)
		if err != nil {
			return false, err
		}
		return won, execSQL(
			ctx,
			tx,
			"UPDATE conversations SET titled_messages = MAX(titled_messages,?) WHERE id = ?",
			seen,
			id,
		)
	})
}

// MarkLive records that the conversation's agent tree was live at `at`
// (`storage.md` § Retiring tool media). A record never moves back, so
// one written late cannot hide a later one.
func (c *ControlService) MarkLive(ctx context.Context, id webapiproto.ConversationID, at types.Timestamp) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) error {
		ms, err := at.Millisecond()
		if err != nil {
			return err
		}
		return execSQL(ctx, tx, "UPDATE conversations SET live_at = MAX(live_at,?) WHERE id = ?", ms, id)
	})
}

// LiveAt returns when the conversation's agent tree was last seen live; none when there
// is no such conversation.
func (c *ControlService) LiveAt(ctx context.Context, id webapiproto.ConversationID) (types.Timestamp, bool, error) {
	var found bool
	record, err := controlCall(
		ctx,
		c,
		func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (types.Timestamp, error) {
			r, ok, err := queryRecord(
				ctx,
				tx,
				"conversations",
				"SELECT live_at FROM conversations WHERE id = ?",
				func(r *storedRow) types.Timestamp {
					return r.instant("live_at")
				},
				id,
			)
			found = ok
			return r, err
		},
	)
	return record, found && err == nil, err
}

// SetWakeup records when the earliest wakeup the conversation's tree saved is due,
// or that it saved none (`runtime.md` § Yield wakeups).
func (c *ControlService) SetWakeup(ctx context.Context, id webapiproto.ConversationID, wakeup WakeupDue) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) error {
		value, err := wakeupColumn(wakeup)
		if err != nil {
			return err
		}
		return execSQL(ctx, tx, "UPDATE conversations SET wakeup_at = ? WHERE id = ?", value, id)
	})
}

// SavedWakeups returns the conversations that are not archived and whose tree saved a
// wakeup, each with its owner and when its earliest wakeup is due,
// earliest first.
func (c *ControlService) SavedWakeups(ctx context.Context) ([]SavedWakeup, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) ([]SavedWakeup, error) {
		return queryRecords(
			ctx,
			tx,
			"conversations",
			"SELECT id,user_id,wakeup_at FROM conversations WHERE wakeup_at IS NOT NULL AND archived = 0 ORDER BY wakeup_at",
			func(r *storedRow) SavedWakeup {
				return SavedWakeup{
					Conversation: checked(r, "id", webapiproto.ParseConversationID),
					Owner:        checked(r, "user_id", webapiproto.ParseUserID),
					Due:          rowWakeup(r, "wakeup_at"),
				}
			},
		)
	})
}

// TouchConversation records activity in the conversation now. Activity never reorders the
// sidebar.
func (c *ControlService) TouchConversation(ctx context.Context, id webapiproto.ConversationID) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, now types.Timestamp) error {
		ms, err := now.Millisecond()
		if err != nil {
			return err
		}
		return execSQL(ctx, tx, "UPDATE conversations SET updated_at = ? WHERE id = ?", ms, id)
	})
}

// AttachedHosts returns the conversation's attached hosts, first attached first.
func (c *ControlService) AttachedHosts(
	ctx context.Context,
	id webapiproto.ConversationID,
) ([]AttachedHostRecord, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) ([]AttachedHostRecord, error) {
		rows, err := attachedRows(ctx, tx, id)
		if err != nil {
			return nil, err
		}
		hosts := make([]AttachedHostRecord, 0, len(rows))
		for _, r := range rows {
			hosts = append(hosts, r.Host)
		}
		return hosts, nil
	})
}

// AttachedHostListing returns the conversation's attached hosts with when each was attached, first
// attached first, as the web app lists them.
func (c *ControlService) AttachedHostListing(
	ctx context.Context,
	id webapiproto.ConversationID,
) ([]AttachedHostListing, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) ([]AttachedHostListing, error) {
		return attachedRows(ctx, tx, id)
	})
}

// SwitchConversationTarget returns the target switch's write, against the target the switch started
// from: false, writing nothing, when the target is no longer
// `expected`, so of two switches from one target exactly one wins. The
// winner records `switch` for every node's next context block and
// advances the execution-context revision.
func (c *ControlService) SwitchConversationTarget(
	ctx context.Context,
	id webapiproto.ConversationID,
	expected webapiproto.ConversationTarget,
	to webapiproto.ConversationTarget,
	switchValue TargetSwitch,
	ends SwitchEnds,
) (bool, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, now types.Timestamp) (bool, error) {
		from := ColumnsForTarget(expected)
		target := ColumnsForTarget(to)
		text, err := encoded(switchValue)
		if err != nil {
			return false, err
		}
		at, err := now.Millisecond()
		if err != nil {
			return false, err
		}
		won, err := affected(
			ctx,
			tx,
			`UPDATE conversations
SET
    target_kind=?2,
    target_device_id=?3,
    target_path=?4,
    target_workspace_id=?5,
    last_switch=?6,
    context_version=context_version+1,
    updated_at=?7
WHERE id=?1
AND target_kind=?8
AND target_device_id IS ?9
AND target_path IS ?10
AND target_workspace_id IS ?11`,
			id,
			target.Kind,
			target.Device,
			target.Path,
			target.Workspace,
			text,
			at,
			from.Kind,
			from.Device,
			from.Path,
			from.Workspace,
		)
		if err != nil || !won {
			return won, err
		}
		if err := switchAttachedHosts(ctx, tx, id, ends, now); err != nil {
			return false, err
		}
		return true, nil
	})
}

// SetAttachedCWD records where the last `demi host shell --host` on the attached
// `device` ended, which is where the next one there starts.
func (c *ControlService) SetAttachedCWD(
	ctx context.Context,
	id webapiproto.ConversationID,
	device webapiproto.DeviceID,
	cwd string,
) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) error {
		return execSQL(
			ctx,
			tx,
			"UPDATE conversation_hosts SET cwd = ? WHERE conversation_id = ? AND device_id = ?",
			cwd,
			id,
			device,
		)
	})
}

const conversationColumns = `conversations.*, COALESCE((
    SELECT revision
    FROM conversation_drafts
    WHERE conversation_id = conversations.id
), 0) AS draft_revision, COALESCE((
 SELECT revision FROM conversation_panels WHERE conversation_id = conversations.id
), 0) AS panel_revision`

func conversationRow(r *storedRow) ConversationRecord {
	var target webapiproto.ConversationTarget
	switch r.text("target_kind") {
	case "cloud":
		target = &webapiproto.ConversationTargetCloud{Path: r.optionalText("target_path")}
	case "device":
		target = &webapiproto.ConversationTargetDevice{
			DeviceID: checked(r, "target_device_id", webapiproto.ParseDeviceID),
			Path:     r.text("target_path"),
		}
	case "workspace":
		target = &webapiproto.ConversationTargetWorkspace{
			WorkspaceID: checked(r, "target_workspace_id", webapiproto.ParseWorkspaceID),
		}
	default:
		r.bad("target_kind", fmt.Errorf("unknown target kind %s", r.text("target_kind")))
	}
	if target != nil {
		r.bad("target_path", webapiproto.ValidateConversationTarget(target))
	}
	return ConversationRecord{
		ID:             checked(r, "id", webapiproto.ParseConversationID),
		Owner:          checked(r, "user_id", webapiproto.ParseUserID),
		Title:          r.text("title"),
		Archived:       r.boolean("archived"),
		Pinned:         r.boolean("pinned"),
		ReadRevision:   r.count("read_revision"),
		Target:         target,
		ContextVersion: r.count("context_version"),
		Model:          optionalJSON(r, "model", types.DecodeModelSelection),
		UserMessages:   r.count("user_messages"),
		TitledMessages: r.count("titled_messages"),
		CreatedAt:      r.instant("created_at"),
		UpdatedAt:      r.instant("updated_at"),
		DraftRevision:  r.count("draft_revision"),
		PanelRevision:  r.count("panel_revision"),
	}
}

func affected(ctx context.Context, tx *sql.Tx, query string, args ...any) (bool, error) {
	result, err := tx.ExecContext(ctx, query, args...)
	if err != nil {
		return false, err
	}
	count, err := result.RowsAffected()
	return count > 0, err
}

func advanceContext(ctx context.Context, tx *sql.Tx, id webapiproto.ConversationID) error {
	return execSQL(ctx, tx, "UPDATE conversations SET context_version = context_version + 1 WHERE id = ?", id)
}

func attachedRows(ctx context.Context, tx *sql.Tx, id webapiproto.ConversationID) ([]AttachedHostListing, error) {
	return queryRecords(
		ctx,
		tx,
		"conversation_hosts",
		"SELECT device_id,name,cwd,attached_at FROM conversation_hosts WHERE conversation_id = ? ORDER BY attached_at,name",
		func(r *storedRow) AttachedHostListing {
			return AttachedHostListing{
				Host: AttachedHostRecord{
					Device: checked(r, "device_id", webapiproto.ParseDeviceID),
					Name:   r.text("name"),
					CWD:    r.optionalText("cwd"),
				},
				At: r.instant("attached_at"),
			}
		},
		id,
	)
}

func wakeupColumn(w WakeupDue) (*int64, error) {
	if w == nil {
		return nil, nil
	}
	switch w := w.(type) {
	case *WakeupAtStart:
		v := int64(0)
		return &v, nil
	case *WakeupAt:
		v, err := w.At.Millisecond()
		return &v, err
	}
	return nil, nil
}

func rowWakeup(r *storedRow, column string) WakeupDue {
	if r.values[column] == nil {
		return nil
	}
	if r.integer(column) == 0 {
		return &WakeupAtStart{}
	}
	return &WakeupAt{At: r.instant(column)}
}

func renameAttachedHost(
	ctx context.Context,
	tx *sql.Tx,
	id webapiproto.ConversationID,
	change *RecordRename,
) error {
	holders, err := queryRecords(
		ctx,
		tx,
		"conversation_hosts",
		"SELECT device_id FROM conversation_hosts WHERE conversation_id = ? AND (device_id = ? OR name = ?)",
		func(r *storedRow) string {
			return r.text("device_id")
		},
		id,
		change.Device,
		change.Name,
	)
	if err != nil {
		return err
	}
	found, taken := false, false
	for _, holder := range holders {
		if holder == string(change.Device) {
			found = true
		} else {
			taken = true
		}
	}
	if !found {
		return ErrNotAttached
	}
	if taken {
		return ErrNameTaken
	}
	err = execSQL(
		ctx,
		tx,
		"UPDATE conversation_hosts SET name = ? WHERE conversation_id = ? AND device_id = ?",
		change.Name,
		id,
		change.Device,
	)
	if err == nil {
		err = advanceContext(ctx, tx, id)
	}
	return err
}

func switchAttachedHosts(
	ctx context.Context,
	tx *sql.Tx,
	id webapiproto.ConversationID,
	ends SwitchEnds,
	now types.Timestamp,
) error {
	if ends.Arriving != nil {
		if err := execSQL(
			ctx,
			tx,
			"DELETE FROM conversation_hosts WHERE conversation_id = ? AND device_id = ?",
			id,
			*ends.Arriving,
		); err != nil {
			return err
		}
	}
	if ends.Departed != nil && (ends.Arriving == nil || ends.Departed.Device != *ends.Arriving) {
		name := string(ends.Departed.Device)
		err := tx.QueryRowContext(ctx, "SELECT name FROM devices WHERE id = ?", ends.Departed.Device).Scan(&name)
		if err != nil && !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		if _, err := InsertAttachedHost(
			ctx,
			tx,
			id,
			AttachedHostRecord{Device: ends.Departed.Device, Name: name, CWD: &ends.Departed.Path},
			now,
		); err != nil {
			return err
		}
	}
	return nil
}
