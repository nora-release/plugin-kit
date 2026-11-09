// Package noraplugin serves Nora's versioned plugin protocol over JSONL stdio.
package noraplugin

import (
	"encoding/json"
	"fmt"
)

// ProtocolVersion is the version a host speaks unless the plugin's manifest
// names another (runtime.protocol); StreamsProtocolVersion adds streams.
const ProtocolVersion uint32 = 1

type InitializeParams struct {
	ProtocolVersion uint32          `json:"protocolVersion"`
	PluginID        string          `json:"pluginId"`
	Config          json.RawMessage `json:"config"`
	GrantedServices []string        `json:"grantedServices"`
	// StreamWindow is, in protocol 2, the host's credit for each stream it
	// reads.
	StreamWindow *uint64 `json:"streamWindow,omitempty"`
}

// InitializeResult's ProtocolVersion and StreamWindow are the server's to
// set: it answers the version the host chose.
type InitializeResult struct {
	ProtocolVersion uint32           `json:"protocolVersion"`
	Tools           []ToolDefinition `json:"tools"`
	Services        []string         `json:"services"`
	StreamWindow    *uint64          `json:"streamWindow,omitempty"`
}

type ToolDefinition struct {
	Name        string          `json:"name"`
	Description string          `json:"description"`
	InputSchema json.RawMessage `json:"inputSchema"`
}

type CallToolParams struct {
	Name      string          `json:"name"`
	Arguments json.RawMessage `json:"arguments"`
}

// callToolParams is tools/call as it comes over the wire.
type callToolParams struct {
	Name         string          `json:"name"`
	Arguments    json.RawMessage `json:"arguments"`
	InputStream  *uint64         `json:"inputStream,omitempty"`
	OutputStream *uint64         `json:"outputStream,omitempty"`
}

type ServiceCallParams struct {
	Service   string          `json:"service"`
	Method    string          `json:"method"`
	Arguments json.RawMessage `json:"arguments"`
}

type ContentBlock struct {
	Type     string `json:"type"`
	Text     string `json:"text,omitempty"`
	Data     string `json:"data,omitempty"`
	MIMEType string `json:"mimeType,omitempty"`
}

func (block ContentBlock) MarshalJSON() ([]byte, error) {
	switch block.Type {
	case "text":
		return json.Marshal(struct {
			Type string `json:"type"`
			Text string `json:"text"`
		}{block.Type, block.Text})
	case "image":
		return json.Marshal(struct {
			Type     string `json:"type"`
			Data     string `json:"data"`
			MIMEType string `json:"mimeType"`
		}{block.Type, block.Data, block.MIMEType})
	default:
		return nil, fmt.Errorf("unsupported content block type %q", block.Type)
	}
}

type ToolResult struct {
	Content []ContentBlock  `json:"content"`
	IsError bool            `json:"isError"`
	Details json.RawMessage `json:"details,omitempty"`
}

func Text(text string) ToolResult {
	return ToolResult{Content: []ContentBlock{{Type: "text", Text: text}}}
}

type RPCError struct {
	Code    int             `json:"code"`
	Message string          `json:"message"`
	Data    json.RawMessage `json:"data,omitempty"`
}

func (err *RPCError) Error() string { return fmt.Sprintf("%s (%d)", err.Message, err.Code) }

type request struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      *uint64         `json:"id,omitempty"`
	Method  string          `json:"method"`
	Params  json.RawMessage `json:"params"`
}

type response struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      *uint64         `json:"id"`
	Result  json.RawMessage `json:"result,omitempty"`
	Error   *RPCError       `json:"error,omitempty"`
}

type serviceCallParams struct {
	Service         string          `json:"service"`
	Method          string          `json:"method"`
	Arguments       json.RawMessage `json:"arguments"`
	ParentRequestID *uint64         `json:"parentRequestId,omitempty"`
	InputStream     *uint64         `json:"inputStream,omitempty"`
	OutputStream    *uint64         `json:"outputStream,omitempty"`
}
