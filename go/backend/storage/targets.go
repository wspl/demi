package storage

import (
	"context"
	"database/sql"
	"encoding/json/v2"
	"errors"
	"fmt"
	"strings"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/webapi"
)

// Temporary: codecs are local until wiregen can load webapi foreign embeddings.

//demi:wire
type AttachedHostRecord struct {
	Device webapi.DeviceID `json:"device" check:"func=webapi.Validate"`
	Name   string          `json:"name" check:"chars=1.."`
	Cwd    *string         `json:"cwd" check:"nullable"`
}
type AttachedHostListing struct {
	Host AttachedHostRecord
	At   core.Timestamp
}

//demi:union tag=kind
type ExecutionTarget interface{ executionTarget() }

//demi:variant cloud
type ExecutionTargetCloud struct {
	DeviceID *webapi.DeviceID `json:"deviceId" check:"nullable,func=webapi.Validate"`
	Path     string           `json:"path"`
}

func (ExecutionTargetCloud) executionTarget() {}

//demi:variant device
type ExecutionTargetDevice struct {
	DeviceID webapi.DeviceID `json:"deviceId" check:"func=webapi.Validate"`
	Path     string          `json:"path"`
}

func (ExecutionTargetDevice) executionTarget() {}

//demi:variant workspace
type ExecutionTargetWorkspace struct {
	WorkspaceID webapi.WorkspaceID `json:"workspaceId" check:"func=webapi.Validate"`
	DeviceID    webapi.DeviceID    `json:"deviceId" check:"func=webapi.Validate"`
	Path        string             `json:"path"`
}

func (ExecutionTargetWorkspace) executionTarget() {}

//demi:wire open
type TargetSwitch struct {
	From ExecutionTarget `json:"from"`
	To   ExecutionTarget `json:"to"`
}
type SettingsChange struct {
	Model                         *webapi.ModelChoice
	ThinkingEffort, ServiceTierID **string
}
type DepartedHost struct {
	Device webapi.DeviceID
	Cwd    string
}
type SwitchEnds struct {
	Departed *DepartedHost
	Arriving *webapi.DeviceID
}

func advanceContext(ctx context.Context, db database, id webapi.ConversationID) error {
	_, err := db.ExecContext(ctx, "UPDATE conversations SET context_version=context_version+1 WHERE id=?", id.String())
	return err
}
func insertAttachedHost(ctx context.Context, db database, id webapi.ConversationID, host AttachedHostRecord, now core.Timestamp) (bool, error) {
	base := strings.TrimSpace(host.Name)
	if base == "" {
		base = host.Device.String()
	}
	rows, err := db.QueryContext(ctx, "SELECT name FROM conversation_hosts WHERE conversation_id=?", id.String())
	if err != nil {
		return false, err
	}
	taken := map[string]bool{}
	for rows.Next() {
		var name string
		if err = rows.Scan(&name); err != nil {
			rows.Close()
			return false, err
		}
		taken[name] = true
	}
	err = rows.Err()
	rows.Close()
	if err != nil {
		return false, err
	}
	candidate := base
	for suffix := 2; taken[candidate]; suffix++ {
		candidate = fmt.Sprintf("%s-%d", base, suffix)
	}
	result, err := db.ExecContext(ctx, "INSERT INTO conversation_hosts(conversation_id,device_id,name,cwd,attached_at) VALUES (?,?,?,?,?) ON CONFLICT(conversation_id,device_id) DO NOTHING", id.String(), host.Device.String(), candidate, host.Cwd, now.Millisecond())
	if err != nil {
		return false, err
	}
	count, err := result.RowsAffected()
	return count == 1, err
}
func (c *Control) AttachedHostListing(ctx context.Context, id webapi.ConversationID) ([]AttachedHostListing, error) {
	rows, err := c.db.QueryContext(ctx, "SELECT device_id,name,cwd,attached_at FROM conversation_hosts WHERE conversation_id=? ORDER BY attached_at,name", id.String())
	if err != nil {
		return nil, sqliteError(err)
	}
	defer rows.Close()
	hosts := []AttachedHostListing{}
	for rows.Next() {
		var entry AttachedHostListing
		var device string
		var ms int64
		if err = rows.Scan(&device, &entry.Host.Name, &entry.Host.Cwd, &ms); err != nil {
			return nil, sqliteError(err)
		}
		entry.Host.Device, err = webapi.ParseDeviceID(device)
		if err != nil {
			return nil, corrupt("conversation_hosts", "device_id", err)
		}
		entry.At, err = instant("conversation_hosts", "attached_at", ms)
		if err != nil {
			return nil, err
		}
		hosts = append(hosts, entry)
	}
	return hosts, sqliteError(rows.Err())
}
func (c *Control) AttachedHosts(ctx context.Context, id webapi.ConversationID) ([]AttachedHostRecord, error) {
	listing, err := c.AttachedHostListing(ctx, id)
	if err != nil {
		return nil, err
	}
	hosts := make([]AttachedHostRecord, len(listing))
	for i, entry := range listing {
		hosts[i] = entry.Host
	}
	return hosts, nil
}
func (c *Control) SetAttachedCwd(ctx context.Context, id webapi.ConversationID, device webapi.DeviceID, cwd string) error {
	_, err := c.db.ExecContext(ctx, "UPDATE conversation_hosts SET cwd=? WHERE conversation_id=? AND device_id=?", cwd, id.String(), device.String())
	return sqliteError(err)
}
func (c *Control) LastSwitch(ctx context.Context, id webapi.ConversationID) (*TargetSwitch, error) {
	var text *string
	err := c.db.QueryRowContext(ctx, "SELECT last_switch FROM conversations WHERE id=?", id.String()).Scan(&text)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, sqliteError(err)
	}
	if text == nil {
		return nil, nil
	}
	var value TargetSwitch
	if err = json.Unmarshal([]byte(*text), &value); err != nil {
		return nil, corrupt("conversations", "last_switch", err)
	}
	return &value, nil
}
func targetColumns(target webapi.ConversationTarget) (kind string, device, path, workspace *string) {
	switch v := target.(type) {
	case webapi.ConversationTargetCloud:
		return "cloud", nil, v.Path, nil
	case webapi.ConversationTargetDevice:
		text := v.DeviceID.String()
		return "device", &text, &v.Path, nil
	case webapi.ConversationTargetWorkspace:
		text := v.WorkspaceID.String()
		return "workspace", nil, nil, &text
	}
	return "", nil, nil, nil
}
func (c *Control) SwitchConversationTarget(ctx context.Context, id webapi.ConversationID, expected, to webapi.ConversationTarget, change TargetSwitch, ends SwitchEnds) (bool, error) {
	document, err := json.Marshal(change)
	if err != nil {
		return false, err
	}
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return false, sqliteError(err)
	}
	defer tx.Rollback()
	kind, device, path, workspace := targetColumns(to)
	fromKind, fromDevice, fromPath, fromWorkspace := targetColumns(expected)
	result, err := tx.ExecContext(ctx, `UPDATE conversations SET target_kind=?,target_device_id=?,target_path=?,target_workspace_id=?,last_switch=?,context_version=context_version+1,updated_at=?
 WHERE id=? AND target_kind=? AND target_device_id IS ? AND target_path IS ? AND target_workspace_id IS ?`, kind, device, path, workspace, string(document), c.clock.Now().Millisecond(), id.String(), fromKind, fromDevice, fromPath, fromWorkspace)
	if err != nil {
		return false, sqliteError(err)
	}
	count, err := result.RowsAffected()
	if err != nil {
		return false, sqliteError(err)
	}
	if count == 0 {
		return false, nil
	}
	if ends.Arriving != nil {
		if _, err = tx.ExecContext(ctx, "DELETE FROM conversation_hosts WHERE conversation_id=? AND device_id=?", id.String(), ends.Arriving.String()); err != nil {
			return false, sqliteError(err)
		}
	}
	if departed := ends.Departed; departed != nil && (ends.Arriving == nil || *ends.Arriving != departed.Device) {
		var name string
		err = tx.QueryRowContext(ctx, "SELECT name FROM devices WHERE id=?", departed.Device.String()).Scan(&name)
		if errors.Is(err, sql.ErrNoRows) {
			name = departed.Device.String()
		} else if err != nil {
			return false, sqliteError(err)
		}
		if _, err = insertAttachedHost(ctx, tx, id, AttachedHostRecord{departed.Device, name, &departed.Cwd}, c.clock.Now()); err != nil {
			return false, sqliteError(err)
		}
	}
	return true, sqliteError(tx.Commit())
}

// RootOf keeps the spelling of the index's conversation identity.
func RootOf(conversation webapi.ConversationID) (core.NodeID, error) {
	return core.ParseNodeID(conversation.String())
}
func ConversationOf(root core.NodeID) (webapi.ConversationID, error) {
	return webapi.ParseConversationID(root.String())
}
