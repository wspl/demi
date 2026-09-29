package webapi

import (
	"github.com/wspl/demi/go/core"
	"regexp"
)

const UUIDPattern = "^([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-8][0-9a-fA-F]{3}-[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}|00000000-0000-0000-0000-000000000000|ffffffff-ffff-ffff-ffff-ffffffffffff)$"
const ExposeIDPattern = "^[a-z2-7]{26}$"

var uuidPattern = regexp.MustCompile(UUIDPattern)
var exposeIDPattern = regexp.MustCompile(ExposeIDPattern)

type UUIDIdentity struct{}

func (UUIDIdentity) CheckIdentity(text string) error {
	if !uuidPattern.MatchString(text) {
		return &InvalidError{Rule: "must be a UUID"}
	}
	return nil
}

type ExposeIdentity struct{}

func (ExposeIdentity) CheckIdentity(text string) error {
	if !exposeIDPattern.MatchString(text) {
		return &InvalidError{Rule: "must be 26 lowercase base32 characters"}
	}
	return nil
}

// UserID is checked on creation and preserves its supplied spelling.
//
//demi:opaque string
//wiregen:browser inline {"type":"string","minLength":1}
type UserID struct{ core.Identity[UserIDKind] }
type UserIDKind struct{ core.NonemptyIdentity }

func ParseUserID(text string) (UserID, error) {
	id, err := core.ParseIdentity[UserIDKind](text)
	return UserID{Identity: id}, err
}
func (v UserID) validate() error { return core.Validate(v.Identity) }

// ProviderID is checked on creation and preserves its supplied spelling.
//
//demi:opaque string
//wiregen:browser inline {"type":"string","minLength":1}
type ProviderID struct{ core.Identity[ProviderIDKind] }
type ProviderIDKind struct{ core.NonemptyIdentity }

func ParseProviderID(text string) (ProviderID, error) {
	id, err := core.ParseIdentity[ProviderIDKind](text)
	return ProviderID{Identity: id}, err
}
func (v ProviderID) validate() error { return core.Validate(v.Identity) }

// CredentialID is checked on creation and preserves its supplied spelling.
//
//demi:opaque string
//wiregen:browser inline {"type":"string","minLength":1}
type CredentialID struct {
	core.Identity[CredentialIDKind]
}
type CredentialIDKind struct{ core.NonemptyIdentity }

func ParseCredentialID(text string) (CredentialID, error) {
	id, err := core.ParseIdentity[CredentialIDKind](text)
	return CredentialID{Identity: id}, err
}
func (v CredentialID) validate() error { return core.Validate(v.Identity) }

// LoginID is checked on creation and preserves its supplied spelling.
//
//demi:opaque string
//wiregen:browser inline {"type":"string","minLength":1}
type LoginID struct{ core.Identity[LoginIDKind] }
type LoginIDKind struct{ core.NonemptyIdentity }

func ParseLoginID(text string) (LoginID, error) {
	id, err := core.ParseIdentity[LoginIDKind](text)
	return LoginID{Identity: id}, err
}
func (v LoginID) validate() error { return core.Validate(v.Identity) }

// DeviceID is checked on creation and preserves its supplied spelling.
//
//demi:opaque string
//wiregen:browser inline {"type":"string","minLength":1}
type DeviceID struct{ core.Identity[DeviceIDKind] }
type DeviceIDKind struct{ core.NonemptyIdentity }

func ParseDeviceID(text string) (DeviceID, error) {
	id, err := core.ParseIdentity[DeviceIDKind](text)
	return DeviceID{Identity: id}, err
}
func (v DeviceID) validate() error { return core.Validate(v.Identity) }

// WorkspaceID is checked on creation and preserves its supplied spelling.
//
//demi:opaque string
//wiregen:browser inline {"type":"string","minLength":1}
type WorkspaceID struct{ core.Identity[WorkspaceIDKind] }
type WorkspaceIDKind struct{ core.NonemptyIdentity }

func ParseWorkspaceID(text string) (WorkspaceID, error) {
	id, err := core.ParseIdentity[WorkspaceIDKind](text)
	return WorkspaceID{Identity: id}, err
}
func (v WorkspaceID) validate() error { return core.Validate(v.Identity) }

// AttachmentID is checked on creation and preserves its supplied spelling.
//
//demi:opaque string
//wiregen:browser inline {"type":"string","minLength":1}
type AttachmentID struct {
	core.Identity[AttachmentIDKind]
}
type AttachmentIDKind struct{ core.NonemptyIdentity }

func ParseAttachmentID(text string) (AttachmentID, error) {
	id, err := core.ParseIdentity[AttachmentIDKind](text)
	return AttachmentID{Identity: id}, err
}
func (v AttachmentID) validate() error { return core.Validate(v.Identity) }

// ConversationID is checked on creation and preserves its supplied spelling.
//
//demi:opaque string
//wiregen:browser inline {"type":"string","pattern":"^([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-8][0-9a-fA-F]{3}-[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}|00000000-0000-0000-0000-000000000000|ffffffff-ffff-ffff-ffff-ffffffffffff)$"}
type ConversationID struct {
	core.Identity[ConversationIDKind]
}
type ConversationIDKind struct{ UUIDIdentity }

func ParseConversationID(text string) (ConversationID, error) {
	id, err := core.ParseIdentity[ConversationIDKind](text)
	return ConversationID{Identity: id}, err
}
func (v ConversationID) validate() error { return core.Validate(v.Identity) }

// OperationID is checked on creation and preserves its supplied spelling.
//
//demi:opaque string
//wiregen:browser inline {"type":"string","pattern":"^([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-8][0-9a-fA-F]{3}-[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}|00000000-0000-0000-0000-000000000000|ffffffff-ffff-ffff-ffff-ffffffffffff)$"}
type OperationID struct{ core.Identity[OperationIDKind] }
type OperationIDKind struct{ UUIDIdentity }

func ParseOperationID(text string) (OperationID, error) {
	id, err := core.ParseIdentity[OperationIDKind](text)
	return OperationID{Identity: id}, err
}
func (v OperationID) validate() error { return core.Validate(v.Identity) }

// ExposeID is checked on creation and preserves its supplied spelling.
//
//demi:opaque string
//wiregen:browser inline {"type":"string","pattern":"^[a-z2-7]{26}$"}
type ExposeID struct{ core.Identity[ExposeIDKind] }
type ExposeIDKind struct{ ExposeIdentity }

func ParseExposeID(text string) (ExposeID, error) {
	id, err := core.ParseIdentity[ExposeIDKind](text)
	return ExposeID{Identity: id}, err
}
func (v ExposeID) validate() error { return core.Validate(v.Identity) }
