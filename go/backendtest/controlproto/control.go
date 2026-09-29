package controlproto

//go:generate go run github.com/wspl/demi/go/cmd/wiregen

import (
	"bytes"
	"encoding/json/jsontext"
	"errors"

	"github.com/wspl/demi/go/internal/wire"
)

// MaxLineBytes is the longest line either end reads, its newline excluded.
const MaxLineBytes = 1 << 20

// The names of the two environment variables that open a test build's hooks.
const (
	// ControlVariable names the control socket's path.
	ControlVariable = "DEMI_TEST_CONTROL"
	// TuningVariable names the tuning file.
	TuningVariable = "DEMI_TEST_TUNING"
)

// An InvalidError says that a message breaks the contract. It names the field
// and the rule, never the value.
type InvalidError = wire.InvalidError

// A Request is what the suite sends: an id the client chooses, which the reply
// names, and the call, whose members (op and params) are the request's own.
//
//demi:wire open
type Request struct {
	ID   string `json:"id" check:"chars=1.."`
	Call Call   `json:",inline"`
}

// A Call is an operation and its parameters.
//
//demi:union tag=op content=params
type Call interface{ call() }

// ClockAdvanceParams moves the manual clock, which a test build runs from its
// start, by ByMs milliseconds, which may be negative. The result is a
// [ClockResult].
//
//demi:variant clock.advance open
type ClockAdvanceParams struct {
	ByMs int64 `json:"byMs"`
}

// ClockSetParams sets the manual clock to AtMs, milliseconds since the Unix
// epoch. The result is a [ClockResult].
//
//demi:variant clock.set open
type ClockSetParams struct {
	AtMs int64 `json:"atMs"`
}

// UsersAddParams adds an account without a route of the API. The result is an
// [IDResult] with the user's id.
//
//demi:variant users.add open
type UsersAddParams struct {
	Email    string `json:"email" check:"chars=1.."`
	Password string `json:"password" check:"chars=1.."`
	Role     string `json:"role" check:"oneof=master|admin|user"`
}

// GateEnterParams enters the file gate of a user's conversation
// (sessions-and-targets.md § Host operations) with a lease of the given
// purpose, and answers once the lease is held: an [IDResult] naming it, which
// the connection releases with [ReleaseParams].
//
//demi:variant gate.enter open
type GateEnterParams struct {
	User         string `json:"user" check:"chars=1.."`
	Conversation string `json:"conversation" check:"chars=1.."`
	Purpose      string `json:"purpose" check:"oneof=demand|maintenance"`
}

// GateWaitingParams answers once at least Count entrants wait at the gate of a
// user's conversation, behind a reservation that holds or waits for it.
//
//demi:variant gate.waiting open
type GateWaitingParams struct {
	User         string `json:"user" check:"chars=1.."`
	Conversation string `json:"conversation" check:"chars=1.."`
	Count        uint64 `json:"count"`
}

// Hold targets: what a hold stops.
const (
	// HoldCommits holds every commit of a conversation's saves: the save has
	// written its rows and waits before its transaction commits.
	HoldCommits = "commits"
	// HoldHelloTokenLookup holds every runner's hello where its token is
	// looked up.
	HoldHelloTokenLookup = "hello:token_lookup"
	// HoldHelloBind holds every runner's hello before it is bound to its
	// device.
	HoldHelloBind = "hello:bind"
	// HoldSyncSnapshot holds every page's synchronization channel once it
	// has read its product state, before it sends it.
	HoldSyncSnapshot = "sync:snapshot"
	// HoldSyncChanges holds every page's synchronization channel once a
	// change woke it, before it reads the parts that changed.
	HoldSyncChanges = "sync:changes"
)

// HoldParams holds a flow from now on until the hold is released: an
// [IDResult] naming it. Whatever passes the held step waits there.
//
//demi:variant hold open
type HoldParams struct {
	Target string `json:"target" check:"oneof=commits|hello:token_lookup|hello:bind|sync:snapshot|sync:changes"`
}

// HoldWaitParams answers once Count passes reached a hold, counting those that
// went away while they waited.
//
//demi:variant hold.wait open
type HoldWaitParams struct {
	Hold  string `json:"hold" check:"chars=1.."`
	Count uint64 `json:"count"`
}

// ReleaseParams ends a hold or a lease of this connection.
//
//demi:variant release open
type ReleaseParams struct {
	ID string `json:"id" check:"chars=1.."`
}

// RetentionRunParams runs a user's retention pass at once and answers once it
// ended (storage.md § The retention pass).
//
//demi:variant retention.run open
type RetentionRunParams struct {
	User string `json:"user" check:"chars=1.."`
}

// ObjectsCountParams reads what reached the object store since the backend
// started: an [ObjectTally].
//
//demi:variant objects.count open
type ObjectsCountParams struct{}

// MailListParams lists the verification mail the backend sent: a [MailList].
//
//demi:variant mail.list open
type MailListParams struct{}

// MailFailParams makes the mail transport fail, or work again.
//
//demi:variant mail.fail open
type MailFailParams struct {
	Failing bool `json:"failing"`
}

func (ClockAdvanceParams) call() {}
func (ClockSetParams) call()     {}
func (UsersAddParams) call()     {}
func (GateEnterParams) call()    {}
func (GateWaitingParams) call()  {}
func (HoldParams) call()         {}
func (HoldWaitParams) call()     {}
func (ReleaseParams) call()      {}
func (RetentionRunParams) call() {}
func (ObjectsCountParams) call() {}
func (MailListParams) call()     {}
func (MailFailParams) call()     {}

// A ClockResult is the manual clock's time after clock.advance or clock.set.
//
//demi:wire open
type ClockResult struct {
	AtMs int64 `json:"atMs"`
}

// An IDResult names a hold or a lease the connection took.
//
//demi:wire open
type IDResult struct {
	ID string `json:"id" check:"chars=1.."`
}

// An ObjectTally is what reached the object store: the puts and the bytes they
// sent, the reads of an object's bytes, the asks whether an object exists, the
// most reads in flight at once, the listings of a prefix and the objects asked
// to be deleted.
//
//demi:wire open
type ObjectTally struct {
	Puts           uint64 `json:"puts"`
	BytesPut       uint64 `json:"bytesPut"`
	Gets           uint64 `json:"gets"`
	Heads          uint64 `json:"heads"`
	MostGetsAtOnce uint64 `json:"mostGetsAtOnce"`
	Lists          uint64 `json:"lists"`
	Deletes        uint64 `json:"deletes"`
}

// Since returns what reached the object store after earlier was read. The most
// reads at once is the tally's own.
func (t ObjectTally) Since(earlier ObjectTally) ObjectTally {
	return ObjectTally{
		Puts:           t.Puts - earlier.Puts,
		BytesPut:       t.BytesPut - earlier.BytesPut,
		Gets:           t.Gets - earlier.Gets,
		Heads:          t.Heads - earlier.Heads,
		MostGetsAtOnce: t.MostGetsAtOnce,
		Lists:          t.Lists - earlier.Lists,
		Deletes:        t.Deletes - earlier.Deletes,
	}
}

// A Mail is a verification code the backend sent.
//
//demi:wire open
type Mail struct {
	Email       string `json:"email" check:"chars=1.."`
	Code        string `json:"code" check:"chars=1.."`
	ExpiresAtMs int64  `json:"expiresAtMs"`
}

// A MailList is the result of mail.list, in the order the mail was sent.
//
//demi:wire open
type MailList struct {
	Mail []Mail `json:"mail"`
}

// A Response is what the backend sends: the reply to a request.
//
//demi:union tag=type
type Response interface{ response() }

// OK is the reply of an operation that succeeded. Result is the operation's
// result as JSON, always written.
//
//demi:variant ok open
type OK struct {
	ID     string         `json:"id" check:"chars=1.."`
	Result jsontext.Value `json:"result"`
}

// Failure is the reply of an operation that failed.
//
//demi:variant error open
type Failure struct {
	ID      string `json:"id" check:"chars=1.."`
	Message string `json:"message"`
}

func (OK) response()      {}
func (Failure) response() {}

// DecodeRequest decodes one request line, its newline removed.
func DecodeRequest(line []byte) (Request, error) {
	return decode[Request](line)
}

// DecodeResponse decodes one response line, its newline removed.
func DecodeResponse(line []byte) (Response, error) {
	return decode[Response](line)
}

// DecodeResult decodes the result of a reply as T, one of the package's result
// types.
func DecodeResult[T any](result jsontext.Value) (T, error) {
	return decode[T](result)
}

// EncodeRequest returns the line that carries request: compact JSON and a
// newline.
func EncodeRequest(request Request) ([]byte, error) {
	if request.Call == nil {
		return nil, errors.New("a request needs a call")
	}
	return encodeLine(request)
}

// EncodeResponse returns the line that carries response: compact JSON and a
// newline.
func EncodeResponse(response Response) ([]byte, error) {
	return encodeLine(response)
}

// EncodeResult returns the JSON of an operation's result, one of the package's
// result types, for a reply.
func EncodeResult[T any](result T) (jsontext.Value, error) {
	return encode(result)
}

// encodeLine is the compact JSON of a message and a newline.
func encodeLine[T any](message T) ([]byte, error) {
	data, err := encode(message)
	if err != nil {
		return nil, err
	}
	return append(bytes.Clone(data), '\n'), nil
}
