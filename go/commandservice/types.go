// Package commandservice speaks the native command protocol over one HTTP/2 connection.
package commandservice

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json/jsontext"
	"maps"
	"path/filepath"
	"strings"

	"github.com/google/jsonschema-go/jsonschema"
	"github.com/nlnwa/whatwg-url/url"
)

const (
	// Version is the supported native command protocol version.
	Version = 1
	// MaxMetadataBytes bounds metadata and service-info payloads.
	MaxMetadataBytes = 256 * 1024
	// MaxRecordBytes bounds one response payload or input chunk.
	MaxRecordBytes = 64 * 1024
	// MaxNumbers bounds one reservation’s count.
	MaxNumbers = 16
	// EditFileBytes bounds copied bytes for one edited file.
	EditFileBytes = 8 * 1024 * 1024
	// EditJobBytes bounds copied bytes for an entire job.
	EditJobBytes = 64 * 1024 * 1024
	// EditJobFiles bounds the files in an edit journal.
	EditJobFiles = 500
	// EditJobSegments bounds the segments in an edit journal.
	EditJobSegments = 1000
)

// CommandCaller identifies the agent or user that started a command.
type CommandCaller struct {
	Kind   string  `json:"kind"`
	Number *uint64 `json:"number,omitzero"`
}

func commandCallerSchema(s *jsonschema.Schema) {
	s.OneOf = []*jsonschema.Schema{
		wireVariant(s, "kind", "agent", []string{"kind", "number"}),
		wireVariant(s, "kind", "user", []string{"kind"}, "number"),
	}
}

// CommandLocale carries an IANA time zone and ordered language preferences.
type CommandLocale struct {
	TimeZone  string   `json:"timeZone"`
	Languages []string `json:"languages"`
}

func commandLocaleSchema(s *jsonschema.Schema) {
	p := s.Properties
	p["timeZone"].MinLength = new(1)
	p["timeZone"].MaxLength = new(64)
	p["languages"].MinItems = new(1)
	p["languages"].MaxItems = new(16)
	p["languages"].Items.MinLength = new(1)
	p["languages"].Items.MaxLength = new(64)
}

// CommandContext carries the trusted conversation, caller, and locale.
type CommandContext struct {
	Conversation string        `json:"conversation"`
	Caller       CommandCaller `json:"caller"`
	Locale       CommandLocale `json:"locale"`
}

func commandContextSchema(s *jsonschema.Schema) {
	p := s.Properties
	p["conversation"].Pattern = conversationPattern
}

// Invocation opens a native command with its arguments and execution context. Args retains
// the caller’s JSON without converting numbers to float64.
type Invocation struct {
	Operation    string            `json:"operation"`
	InvocationID string            `json:"invocationId"`
	Context      CommandContext    `json:"context"`
	Args         jsontext.Value    `json:"args"`
	Cwd          string            `json:"cwd"`
	Env          map[string]string `json:"env"`
	Edits        *EditContext      `json:"edits,omitzero"`
	JSON         *bool             `json:"json,omitzero"`
}

func invocationSchema(s *jsonschema.Schema) {
	p := s.Properties
	for _, name := range []string{"operation", "invocationId", "cwd"} {
		p[name].MinLength = new(1)
	}
	p["cwd"].Pattern = noNULPattern
	p["env"].PropertyNames = &jsonschema.Schema{Type: "string", Pattern: `^[^\x00=]+$`}
	p["env"].AdditionalProperties.Pattern = noNULPattern
}

// validate checks the invocation’s optional edit paths against the running platform.
func (v Invocation) validate() error {
	if v.Edits != nil {
		return v.Edits.validate()
	}
	return nil
}

// CommandError describes a command failure in the command’s own words.
type CommandError struct {
	Code    string `json:"code"`
	Message string `json:"message"`
}

// Completion ends an invocation with an exit code and optional command error.
type Completion struct {
	ExitCode uint8         `json:"exitCode"`
	Error    *CommandError `json:"error,omitzero"`
}

// validate needs no checks beyond the completion schema’s exit-code and error fields.
func (v Completion) validate() error {
	return nil
}

// ConversationRequest requests status or release of trusted conversation state.
type ConversationRequest struct {
	Operation    string `json:"operation"`
	Conversation string `json:"conversation,omitzero"`
}

func conversationRequestSchema(s *jsonschema.Schema) {
	p := s.Properties
	p["conversation"].Pattern = conversationPattern
	s.OneOf = []*jsonschema.Schema{
		wireVariant(s, "operation", "status", []string{"operation"}, "conversation"),
		wireVariant(s, "operation", "release", []string{"operation", "conversation"}),
	}
}

// validate needs no checks beyond the schema’s status/release variants and conversation identity.
func (v ConversationRequest) validate() error {
	return nil
}

// ConversationStatus lists the conversations for which a service retains resources.
type ConversationStatus struct {
	Conversations []string `json:"conversations"`
}

func conversationStatusSchema(s *jsonschema.Schema) {
	p := s.Properties
	p["conversations"].Items.MinLength = new(1)
}

// validate needs no checks beyond the schema’s conversation-name list.
func (v ConversationStatus) validate() error {
	return nil
}

// Sequence identifies a conversation-owned number sequence.
type Sequence string

func sequenceSchema(s *jsonschema.Schema) {
	s.Enum = []any{Tab}
}

// Tab is the browser-tab sequence.
const Tab Sequence = "tab"

// NumbersOpen is the empty metadata that opens the numbers stream.
type NumbersOpen struct{}

// validate needs no checks beyond the empty numbers-open metadata schema.
func (v NumbersOpen) validate() error {
	return nil
}

// NumbersRequest reserves a bounded range from a conversation’s sequence.
type NumbersRequest struct {
	ID           uint64   `json:"id"`
	Conversation string   `json:"conversation"`
	Sequence     Sequence `json:"sequence"`
	Count        int      `json:"count"`
}

func numbersRequestSchema(s *jsonschema.Schema) {
	p := s.Properties
	p["conversation"].Pattern = conversationPattern
	p["count"].Minimum = new(float64(1))
	p["count"].Maximum = new(float64(MaxNumbers))
}

// validate needs no checks beyond the schema’s sequence, conversation, and reservation bounds.
func (v NumbersRequest) validate() error {
	return nil
}

// NumbersAnswer carries either the first reserved number or an error for one request.
type NumbersAnswer struct {
	ID    uint64  `json:"id"`
	First *uint64 `json:"first,omitzero"`
	Error *string `json:"error,omitzero"`
}

func numbersAnswerSchema(s *jsonschema.Schema) {
	p := s.Properties
	p["first"].Minimum = new(float64(1))
	p["error"].MinLength = new(1)
}

// validate requires exactly one reservation outcome: a first number or an error.
func (v NumbersAnswer) validate() error {
	if (v.First == nil) == (v.Error == nil) {
		return &InvalidError{Field: "first/error", Rule: "a numbers answer carries either its first number or its error"}
	}
	return nil
}

// PackageArtifact identifies one executable by SHA-256 digest and byte size.
type PackageArtifact struct {
	SHA256 string `json:"sha256"`
	Size   uint64 `json:"size"`
}

func packageArtifactSchema(s *jsonschema.Schema) {
	p := s.Properties
	p["sha256"].Pattern = `^[0-9a-f]{64}$`
	p["size"].Minimum = new(float64(1))
	p["size"].Maximum = new(float64(1<<53 - 1))
}

// PackageDescriptor identifies an immutable release, its operations, and its artifacts.
type PackageDescriptor struct {
	ID              string                     `json:"id"`
	Version         string                     `json:"version"`
	ProtocolVersion uint64                     `json:"protocolVersion"`
	Operations      []string                   `json:"operations"`
	Targets         map[string]PackageArtifact `json:"targets"`
}

func packageDescriptorSchema(s *jsonschema.Schema) {
	p := s.Properties
	p["id"].Pattern = `^[a-z0-9]+(?:[.-][a-z0-9]+)+$`
	p["version"].MinLength = new(1)
	serviceCatalogSchema(s)
	var targets []any
	for _, target := range Targets() {
		targets = append(targets, target)
	}
	p["targets"].PropertyNames = &jsonschema.Schema{Enum: targets}
}

// validate needs no checks beyond the descriptor schema’s identity, catalog, and target constraints.
func (v PackageDescriptor) validate() error {
	return nil
}

// ServiceInfo reports a service’s protocol version and operation catalog.
type ServiceInfo struct {
	ProtocolVersion uint64   `json:"protocolVersion"`
	Operations      []string `json:"operations"`
}

func serviceInfoSchema(s *jsonschema.Schema) {
	serviceCatalogSchema(s)
}

// validate needs no checks beyond the schema’s protocol version and unique operation catalog.
func (v ServiceInfo) validate() error {
	return nil
}

// ArtifactURL locates an artifact at an HTTP or HTTPS URL without credentials.
type ArtifactURL struct {
	URL       string `json:"url"`
	ExpiresAt *int64 `json:"expiresAt,omitzero"`
}

// validate requires a WHATWG HTTP or HTTPS URL without embedded credentials.
func (v ArtifactURL) validate() error {
	parsed, err := url.Parse(v.URL)
	if err != nil || (parsed.Scheme() != "http" && parsed.Scheme() != "https") || parsed.Username() != "" || parsed.Password() != "" {
		return &InvalidError{Field: "url", Rule: "artifact downloads require an HTTP or HTTPS URL without credentials"}
	}
	return nil
}

// ArtifactPath locates an artifact on the runner’s filesystem.
type ArtifactPath struct {
	Path string `json:"path"`
}

func artifactPathSchema(s *jsonschema.Schema) {
	p := s.Properties
	p["path"].MinLength = new(1)
}

// validate needs no checks beyond the schema’s nonempty artifact path.
func (v ArtifactPath) validate() error {
	return nil
}

// ArtifactLocation holds either an artifact URL, with optional expiration, or a local path.
type ArtifactLocation struct {
	URL       string `json:"url,omitzero"`
	ExpiresAt *int64 `json:"expiresAt,omitzero"`
	Path      string `json:"path,omitzero"`
}

func artifactLocationSchema(s *jsonschema.Schema) {
	u := s.CloneSchemas()
	delete(u.Properties, "path")
	u.Required = []string{"url"}
	q := s.CloneSchemas()
	delete(q.Properties, "url")
	delete(q.Properties, "expiresAt")
	q.Required = []string{"path"}
	q.Properties["path"].MinLength = new(1)
	s.OneOf = []*jsonschema.Schema{u, q}
}

// validate checks the URL alternative for an HTTP or HTTPS address without credentials.
func (v ArtifactLocation) validate() error {
	if v.Path == "" {
		return (ArtifactURL{URL: v.URL, ExpiresAt: v.ExpiresAt}).validate()
	}
	return nil
}

// EditContext names the absolute edit-journal directory and its writer lock.
type EditContext struct {
	Directory string `json:"directory"`
	Lock      string `json:"lock"`
}

// validate requires absolute edit-directory and lock paths without NUL on this platform.
func (v EditContext) validate() error {
	for name, path := range map[string]string{"directory": v.Directory, "lock": v.Lock} {
		if !filepath.IsAbs(path) || strings.ContainsRune(path, 0) {
			return &InvalidError{Field: name, Rule: "must be an absolute path without NUL"}
		}
	}
	return nil
}

// EditCopies names the optional before and after copies of one edit segment.
type EditCopies struct {
	Original *string `json:"original,omitzero"`
	Modified *string `json:"modified,omitzero"`
}

func editCopiesSchema(s *jsonschema.Schema) {
	p := s.Properties
	for _, name := range []string{"original", "modified"} {
		p[name].MinLength = new(1)
		p[name].Pattern = noNULPattern
	}
}

// EditKind records whether a file was added or already existed.
type EditKind string

func editKindSchema(s *jsonschema.Schema) {
	s.Enum = []any{Added, Modified}
}

const (
	// Added marks a file absent before the job.
	Added EditKind = "added"
	// Modified marks a file present before the job.
	Modified EditKind = "modified"
)

// EditFile records a changed file and its edit segments.
type EditFile struct {
	Path  string       `json:"path"`
	Kind  EditKind     `json:"kind"`
	Edits []EditCopies `json:"edits"`
}

func editFileSchema(s *jsonschema.Schema) {
	p := s.Properties
	p["path"].MinLength = new(1)
	p["path"].Pattern = noNULPattern
	p["edits"].MaxItems = new(EditJobSegments)
}

// EditJournal records the bounded set of files and copies made by a job.
type EditJournal struct {
	Files          []EditFile `json:"files"`
	BytesCopied    uint64     `json:"bytesCopied"`
	NextSegment    uint64     `json:"nextSegment"`
	FilesTruncated bool       `json:"filesTruncated"`
}

func editJournalSchema(s *jsonschema.Schema) {
	p := s.Properties
	p["files"].MaxItems = new(EditJobFiles)
	p["bytesCopied"].Maximum = new(float64(EditJobBytes))
	p["nextSegment"].Maximum = new(float64(EditJobSegments))
}

// validate needs no checks beyond the schema’s file, byte, and segment bounds.
func (v EditJournal) validate() error {
	return nil
}

const (
	targetDarwinARM64  = "aarch64-apple-darwin"
	targetDarwinAMD64  = "x86_64-apple-darwin"
	targetLinuxARM64   = "aarch64-unknown-linux-musl"
	targetLinuxAMD64   = "x86_64-unknown-linux-musl"
	targetWindowsARM64 = "aarch64-pc-windows-msvc"
	targetWindowsAMD64 = "x86_64-pc-windows-msvc"
)

// Targets returns the supported native artifact target triples.
func Targets() []string {
	return []string{
		targetDarwinARM64, targetDarwinAMD64,
		targetLinuxARM64, targetLinuxAMD64,
		targetWindowsARM64, targetWindowsAMD64,
	}
}

// Digest returns the SHA-256 of the descriptor’s RFC 8785 canonical JSON.
func (p PackageDescriptor) Digest() (string, error) {
	b, err := Encode(p)
	if err != nil {
		return "", err
	}
	v := jsontext.Value(b)
	if err := v.Canonicalize(); err != nil {
		return "", err
	}
	sum := sha256.Sum256(v)
	return hex.EncodeToString(sum[:]), nil
}

// Serves reports whether a service offers the descriptor's protocol and operation set.
func (p PackageDescriptor) Serves(info ServiceInfo) bool {
	declared := make(map[string]struct{})
	served := make(map[string]struct{})
	for _, operation := range p.Operations {
		declared[operation] = struct{}{}
	}
	for _, operation := range info.Operations {
		served[operation] = struct{}{}
	}
	return p.ProtocolVersion == info.ProtocolVersion && maps.Equal(declared, served)
}

// Target returns the executable artifact for target, or an error when it is absent.
func (p PackageDescriptor) Target(target string) (PackageArtifact, error) {
	a, ok := p.Targets[target]
	if !ok {
		return a, &InvalidError{Field: "targets", Rule: "missing target " + target}
	}
	return a, nil
}
