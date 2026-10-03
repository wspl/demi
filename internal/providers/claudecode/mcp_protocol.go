package claudecode

import (
	"crypto/rand"
	"encoding/json"
	"fmt"
	"slices"
	"strings"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/version"
)

// These are the revisions understood by the reference's rmcp 3.4.1 server.
var mcpVersions = []string{"2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25", "2026-07-28"}

type mcpPhase uint8

const (
	mcpInitial mcpPhase = iota
	mcpSession
	mcpInline
	mcpEnded
)

type mcpImplementation struct {
	Name    string `json:"name"`
	Version string `json:"version"`
}

// The MCP identity carries the executable release version.
var mcpIdentity = mcpImplementation{Name: "demi", Version: version.Release}

type initializeParams struct {
	Version      string                     `json:"protocolVersion"`
	Capabilities map[string]json.RawMessage `json:"capabilities"`
	Client       mcpImplementation          `json:"clientInfo"`
}

type requestMetadata struct {
	Version      provider.ReportedString                       `json:"io.modelcontextprotocol/protocolVersion"    wire:"optional"`
	Capabilities provider.Reported[map[string]json.RawMessage] `json:"io.modelcontextprotocol/clientCapabilities" wire:"optional"`
}

// admit applies the reference SDK's handshake and inline metadata rules.
// A rejected first request ends its MCP server; later requests cannot revive it.
func (m *mcpServer) admit(method string, params json.RawMessage, initialize bool) (bool, *mcpError) {
	if initialize || m.phase == mcpInitial && method == "ping" {
		return false, nil
	}
	// An absent or unreadable metadata object has no usable protocol fields.
	envelope, _ := provider.DecodeUntagged[struct {
		Meta *requestMetadata `json:"_meta"`
	}](string(params))
	meta := requestMetadata{}
	if envelope.Meta != nil {
		meta = *envelope.Meta
	}
	if m.phase == mcpInitial {
		if failure := missingMetadata(meta); failure != nil {
			m.phase = mcpEnded
			m.failure = "expect initialized request, but received a request without required metadata"
			return false, failure
		}
		m.phase = mcpInline
	}
	if meta.Version.Value != nil && !slices.Contains(mcpVersions, *meta.Version.Value) {
		return false, &mcpError{
			Code:    -32022,
			Message: "Unsupported protocol version",
			Data:    &unsupportedVersion{*meta.Version.Value, mcpVersions},
		}
	}
	modern := meta.Version.Value != nil && *meta.Version.Value >= "2026-07-28"
	if m.phase == mcpInline || method == "discover" || modern {
		if failure := missingMetadata(meta); failure != nil {
			return modern, failure
		}
	}
	return modern, nil
}

func missingMetadata(meta requestMetadata) *mcpError {
	var missing []string
	if meta.Version.Value == nil {
		missing = append(missing, "io.modelcontextprotocol/protocolVersion")
	}
	if meta.Capabilities.Value == nil {
		missing = append(missing, "io.modelcontextprotocol/clientCapabilities")
	}
	if len(missing) == 0 {
		return nil
	}
	return &mcpError{
		Code: -32602,
		Message: "request _meta is missing or has malformed required fields: " +
			strings.Join(missing, ", "),
	}
}

// emptyCatalog supplies the reference SDK's empty directories for capabilities
// Demi does not advertise. Malformed pagination is an unrecognized request.
func emptyCatalog(method string, params json.RawMessage) (setModerner, bool) {
	var result setModerner
	switch method {
	case "resources/list":
		result = &resourcesResult{Resources: []struct{}{}}
	case "resources/templates/list":
		result = &templatesResult{Templates: []struct{}{}}
	case "prompts/list":
		result = &promptsResult{Prompts: []struct{}{}}
	default:
		return nil, false
	}
	if len(params) > 0 && string(params) != "null" {
		if _, err := provider.DecodeUntagged[struct {
			Cursor *string `json:"cursor"`
		}](string(params)); err != nil {
			return nil, false
		}
	}
	return result, true
}

// completion reads only the reference SDK's declared completion arguments.
func completion(params json.RawMessage) (setModerner, bool) {
	request, err := provider.DecodeUntagged[struct {
		Ref      json.RawMessage `json:"ref"`
		Argument struct {
			Name  string `json:"name"`
			Value string `json:"value"`
		} `json:"argument"`
		Context *struct {
			Arguments *map[string]string `json:"arguments"`
		} `json:"context"`
	}](string(params))
	if err != nil {
		return nil, false
	}
	ref, err := provider.DecodeTagged(string(request.Ref), map[string]func(string) (bool, error){
		"ref/resource": func(text string) (bool, error) {
			_, err := provider.DecodeUntagged[struct {
				URI string `json:"uri"`
			}](text)
			return true, err
		},
		"ref/prompt": func(text string) (bool, error) {
			_, err := provider.DecodeUntagged[struct {
				Name  string  `json:"name"`
				Title *string `json:"title"`
			}](text)
			return true, err
		},
	})
	if err != nil || ref == nil {
		return nil, false
	}
	return &completionResult{Completion: completionInfo{Values: []string{}}}, true
}

// controlID names a CLI initialization or a tool call the CLI did not name.
func controlID() string {
	var id [16]byte
	// crypto/rand.Read fills the buffer or terminates the process; it never returns an error.
	_, _ = rand.Read(id[:])
	id[6] = (id[6] & 0x0f) | 0x40
	id[8] = (id[8] & 0x3f) | 0x80
	return fmt.Sprintf("%x-%x-%x-%x-%x", id[:4], id[4:6], id[6:8], id[8:10], id[10:])
}
