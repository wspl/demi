package plugin

import (
	"encoding/json"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/webapi"
)

//revive:disable:exported
// Contract doc comments describe the wire value; they do not start with the type's name.

// One operation a plugin asks of Demi.
// +demi:root
// +demi:union tag=type
//
//sumtype:decl
type PortMessage interface{ portMessage() }

// An operation of a command request's rpc port: its IO and the
// invoking node's command storage.
// +demi:variant PortMessage rpc
type PortMessageRPC struct {
	Request host.PortRequest `json:"request"`
}

func (*PortMessageRPC) portMessage() {}

// The plugin's value `key` for the user.
// +demi:variant PortMessage read_value
type PortMessageReadValue struct {
	Key string `json:"key"`
}

func (*PortMessageReadValue) portMessage() {}

// Every value of the plugin's for the user.
// +demi:variant PortMessage list_values
type PortMessageListValues struct{}

func (*PortMessageListValues) portMessage() {}

// Writes `key` if its revision is still `revision`; none for a value
// that does not exist yet. `blobs` are the blobs the value names, which
// stay while it names them.
// +demi:variant PortMessage write_value
type PortMessageWriteValue struct {
	Key   string          `json:"key"`
	Value json.RawMessage `json:"value"`
	// +demi:nullable
	Revision *uint64        `json:"revision,omitempty"`
	Blobs    []core.BlobRef `json:"blobs,omitempty"`
}

func (*PortMessageWriteValue) portMessage() {}

// Removes `key` if its revision is still `revision`.
// +demi:variant PortMessage remove_value
type PortMessageRemoveValue struct {
	Key      string `json:"key"`
	Revision uint64 `json:"revision"`
}

func (*PortMessageRemoveValue) portMessage() {}

// Stores `bytes` in the user's blob namespace.
// +demi:variant PortMessage put_blob
type PortMessagePutBlob struct {
	Bytes core.B64Bytes `json:"bytes"`
}

func (*PortMessagePutBlob) portMessage() {}

// The bytes of the user's blob `blob`.
// +demi:variant PortMessage get_blob
type PortMessageGetBlob struct {
	Blob core.BlobRef `json:"blob"`
}

func (*PortMessageGetBlob) portMessage() {}

// Replaces the plugin's set of Host directories for the user.
// +demi:variant PortMessage set_directories
type PortMessageSetDirectories struct {
	Directories []HostDirectory `json:"directories"`
}

func (*PortMessageSetDirectories) portMessage() {}

// Reads paths on the request's conversation's main Host, if it is
// running, without waking it.
// +demi:variant PortMessage read_host_files
type PortMessageReadHostFiles struct {
	Reads []HostRead `json:"reads"`
}

func (*PortMessageReadHostFiles) portMessage() {}

// The plugin's page state of `scope` changed: the user's, or the
// request's conversation's.
// +demi:variant PortMessage changed
type PortMessageChanged struct {
	Scope Scope `json:"scope"`
}

func (*PortMessageChanged) portMessage() {}

// Runs one operation of a package the plugin's commands bind, on the
// request's conversation's main Host.
// +demi:variant PortMessage package_call
// +demi:check validatePackageCall
type PortMessagePackageCall struct {
	Operation declare.NativeOperation `json:"operation"`
	Args      json.RawMessage         `json:"args"`
	Kind      CallKind                `json:"kind"`
}

func (*PortMessagePackageCall) portMessage() {}

// The request's conversation's main and attached Hosts.
// +demi:variant PortMessage conversation_hosts
type PortMessageConversationHosts struct{}

func (*PortMessageConversationHosts) portMessage() {}

// The user's live exposes, soonest expiry first.
// +demi:variant PortMessage list_exposes
type PortMessageListExposes struct{}

func (*PortMessageListExposes) portMessage() {}

// A new expose of `address` on the user's `device`, for `lifetime`
// seconds.
// +demi:variant PortMessage create_expose
type PortMessageCreateExpose struct {
	Device   webapi.DeviceID `json:"device"`
	Address  string          `json:"address"`
	Lifetime uint64          `json:"lifetime"`
}

func (*PortMessageCreateExpose) portMessage() {}

// Moves the expose's expiry to `lifetime` seconds from now.
// +demi:variant PortMessage renew_expose
type PortMessageRenewExpose struct {
	Expose   webapi.ExposeID `json:"expose"`
	Lifetime uint64          `json:"lifetime"`
}

func (*PortMessageRenewExpose) portMessage() {}

// Destroys the expose at once.
// +demi:variant PortMessage remove_expose
type PortMessageRemoveExpose struct {
	Expose webapi.ExposeID `json:"expose"`
}

func (*PortMessageRemoveExpose) portMessage() {}

// The answer to one [PortMessage].
// +demi:root
// +demi:union tag=type
//
//sumtype:decl
type PortAnswer interface{ portAnswer() }

// +demi:variant PortAnswer rpc
type PortAnswerRPC struct {
	Response host.PortResponse `json:"response"`
}

func (*PortAnswerRPC) portAnswer() {}

// +demi:variant PortAnswer value
type PortAnswerValue struct {
	// +demi:nullable
	Value *StoredValue `json:"value,omitempty"`
}

func (*PortAnswerValue) portAnswer() {}

// +demi:variant PortAnswer values
type PortAnswerValues struct {
	Values map[string]StoredValue `json:"values"`
}

func (*PortAnswerValues) portAnswer() {}

// The value's new revision.
// +demi:variant PortAnswer written
type PortAnswerWritten struct {
	Revision uint64 `json:"revision"`
}

func (*PortAnswerWritten) portAnswer() {}

// The name of a blob the plugin put.
// +demi:variant PortAnswer blob
type PortAnswerBlob struct {
	Blob core.BlobRef `json:"blob"`
}

func (*PortAnswerBlob) portAnswer() {}

// A blob's bytes; none for a blob the user's namespace lacks.
// +demi:variant PortAnswer bytes
type PortAnswerBytes struct {
	// +demi:nullable
	Bytes *core.B64Bytes `json:"bytes,omitempty"`
}

func (*PortAnswerBytes) portAnswer() {}

// Each directory's path on every Host, in the order of the set.
// +demi:variant PortAnswer directories
type PortAnswerDirectories struct {
	Paths []DirectoryPath `json:"paths"`
}

func (*PortAnswerDirectories) portAnswer() {}

// What each read found, in the order of the reads.
// +demi:variant PortAnswer host_files
type PortAnswerHostFiles struct {
	Files []HostFile `json:"files"`
}

func (*PortAnswerHostFiles) portAnswer() {}

// The operation is done and answers nothing.
// +demi:variant PortAnswer done
type PortAnswerDone struct{}

func (*PortAnswerDone) portAnswer() {}

// A package call's JSON result.
// +demi:variant PortAnswer called
type PortAnswerCalled struct {
	Result json.RawMessage `json:"result"`
}

func (*PortAnswerCalled) portAnswer() {}

// +demi:variant PortAnswer hosts
type PortAnswerHosts struct {
	Hosts []ConversationHost `json:"hosts"`
}

func (*PortAnswerHosts) portAnswer() {}

// +demi:variant PortAnswer exposes
type PortAnswerExposes struct {
	List ExposeList `json:"list"`
}

func (*PortAnswerExposes) portAnswer() {}

// +demi:variant PortAnswer expose
type PortAnswerExpose struct {
	Expose ExposeRecord `json:"expose"`
}

func (*PortAnswerExpose) portAnswer() {}

// +demi:variant PortAnswer refused
type PortAnswerRefused struct {
	Refusal PortRefusal `json:"refusal"`
}

func (*PortAnswerRefused) portAnswer() {}

// A stored value and the revision a write of it names.
// +demi:root
type StoredValue struct {
	Value    json.RawMessage `json:"value"`
	Revision uint64          `json:"revision"`
}

// A directory the plugin keeps on every Host its user's jobs run on
// (`plugins.md` § Host directories).
// +demi:root
type HostDirectory struct {
	// 1 to 64 lowercase letters, digits and hyphens, unique in the set.
	Name  string          `json:"name"`
	Files []DirectoryFile `json:"files"`
}

// A file of a [HostDirectory], whose bytes are a blob the plugin put.
// +demi:root
type DirectoryFile struct {
	// Relative, with `/` between its parts.
	Path       string       `json:"path"`
	Executable bool         `json:"executable"`
	Blob       core.BlobRef `json:"blob"`
}

// Where a directory of the set is on every Host.
// +demi:root
type DirectoryPath struct {
	Name string `json:"name"`
	// Below the Host's home, as `~/.demi/plugins/skills/tdd-9f2c1a7b3e40`.
	Path string `json:"path"`
}

// One path a plugin reads on a Host: absolute, as the Host names it, and
// the most bytes of a file to answer.
// +demi:root
type HostRead struct {
	Path  string `json:"path"`
	Limit uint64 `json:"limit"`
}

// What a path on a Host is. A symbolic link is followed.
// +demi:root
// +demi:union tag=type
//
//sumtype:decl
type HostFile interface{ hostFile() }

// +demi:variant HostFile missing
type HostFileMissing struct{}

func (*HostFileMissing) hostFile() {}

// A directory's entries, in the Host's order.
// +demi:variant HostFile directory
type HostFileDirectory struct {
	Entries []HostEntry `json:"entries"`
}

func (*HostFileDirectory) hostFile() {}

// A file's first bytes, at most the read's limit, and its size.
// +demi:variant HostFile file
type HostFileFile struct {
	Bytes core.B64Bytes `json:"bytes"`
	Size  uint64        `json:"size"`
}

func (*HostFileFile) hostFile() {}

// Neither a file nor a directory.
// +demi:variant HostFile other
type HostFileOther struct{}

func (*HostFileOther) hostFile() {}

// The Host could not read it, such as for want of permission.
// +demi:variant HostFile unreadable
type HostFileUnreadable struct {
	Message string `json:"message"`
}

func (*HostFileUnreadable) hostFile() {}

// An entry of a directory on a Host.
// +demi:root
type HostEntry struct {
	Name string    `json:"name"`
	Kind EntryKind `json:"kind"`
}

// An entry's kind, without following a symbolic link.
// +demi:root
// +demi:enum file directory symlink other
type EntryKind string

const (
	EntryKindFile      EntryKind = "file"
	EntryKindDirectory EntryKind = "directory"
	EntryKindSymlink   EntryKind = "symlink"
	EntryKindOther     EntryKind = "other"
)

// What a package call does on the Host, which decides whether it wakes a
// stopped Cloud and whether it is activity (`resource-lifecycle.md`
// § Activity).
// +demi:root
// +demi:enum starts operates looks
type CallKind string

const (
	// Work the user starts, such as opening a tab: it wakes a stopped
	// Cloud.
	CallKindStarts CallKind = "starts"
	// An operation on what runs there, such as closing a tab: activity,
	// but a stopped Cloud is refused rather than woken.
	CallKindOperates CallKind = "operates"
	// A look, such as listing the tabs: a stopped Cloud is refused, and
	// the look is no activity.
	CallKindLooks CallKind = "looks"
)

// A Host of the request's conversation.
// +demi:root
type ConversationHost struct {
	// Its name as `demi host list` shows it.
	Name   string          `json:"name"`
	Device webapi.DeviceID `json:"device"`
	Role   HostRole        `json:"role"`
	Online bool            `json:"online"`
}

// +demi:root
// +demi:enum main attached
type HostRole string

const (
	HostRoleMain     HostRole = "main"
	HostRoleAttached HostRole = "attached"
)

// The user's live exposes.
// +demi:root
type ExposeList struct {
	// Whether the instance has an expose domain; without one there are no
	// exposes.
	Available bool `json:"available"`
	// When Demi listed them, by the clock their expiries are read by.
	ListedAt core.Timestamp `json:"listedAt"`
	// Soonest expiry first.
	Exposes []ExposeRecord `json:"exposes"`
}

// An expose as the port shows it (`expose.md` § The expose record).
// +demi:root
type ExposeRecord struct {
	ID     webapi.ExposeID `json:"id"`
	Device webapi.DeviceID `json:"device"`
	// The device's name, the Cloud's as `Cloud`.
	DeviceName string               `json:"deviceName"`
	Address    webapi.ExposeAddress `json:"address"`
	URL        string               `json:"url"`
	CreatedAt  core.Timestamp       `json:"createdAt"`
	ExpiresAt  core.Timestamp       `json:"expiresAt"`
}

// Why Demi refused a port operation.
// +demi:root
// +demi:union tag=type
//
//sumtype:decl
type PortRefusal interface {
	portRefusal()
	error
}

// The conversation's host access refused, as its routes would: a page
// call that passes it on answers with its code and status.
// +demi:variant PortRefusal host
type PortRefusalHost struct {
	Code    webapi.ErrorCode `json:"code"`
	Status  uint16           `json:"status"`
	Message string           `json:"message"`
}

func (*PortRefusalHost) portRefusal() {}

// The package operation exited nonzero, with what it wrote to its
// standard error.
// +demi:variant PortRefusal operation
type PortRefusalOperation struct {
	Stderr string `json:"stderr"`
}

func (*PortRefusalOperation) portRefusal() {}

// Another write of the value came first.
// +demi:variant PortRefusal conflict
type PortRefusalConflict struct{}

func (*PortRefusalConflict) portRefusal() {}

// An expose operation was refused.
// +demi:variant PortRefusal expose
type PortRefusalExpose struct {
	Reason  ExposeRefusal `json:"reason"`
	Message string        `json:"message"`
}

func (*PortRefusalExpose) portRefusal() {}

// The operation needs a conversation, and the request has none.
// +demi:variant PortRefusal no_conversation
type PortRefusalNoConversation struct{}

func (*PortRefusalNoConversation) portRefusal() {}

// The conversation's main Host is not running: a stopped Cloud, or a
// device whose runner is not connected. A read never wakes it.
// +demi:variant PortRefusal not_running
type PortRefusalNotRunning struct{}

func (*PortRefusalNotRunning) portRefusal() {}

// +demi:root
// +demi:enum unavailable invalid_address device_not_found device_offline not_found
type ExposeRefusal string

const (
	// The instance has no expose domain.
	ExposeRefusalUnavailable ExposeRefusal = "unavailable"
	// The address is not `host:port` or a port.
	ExposeRefusalInvalidAddress ExposeRefusal = "invalid_address"
	// The device is not the user's.
	ExposeRefusalDeviceNotFound ExposeRefusal = "device_not_found"
	// The device's runner is not connected, or its Cloud is not running.
	ExposeRefusalDeviceOffline ExposeRefusal = "device_offline"
	// The user has no live expose of that id.
	ExposeRefusalNotFound ExposeRefusal = "not_found"
)
