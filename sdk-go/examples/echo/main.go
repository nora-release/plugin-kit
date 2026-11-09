package main

import (
	"context"
	"encoding/json"
	"fmt"
	"os"

	plugin "github.com/nora-release/plugin-kit/sdk-go"
)

type echo struct{}

func (echo) Initialize(context.Context, plugin.InitializeParams) (plugin.InitializeResult, error) {
	return plugin.InitializeResult{ProtocolVersion: plugin.ProtocolVersion, Services: []string{"example.echo"}, Tools: []plugin.ToolDefinition{{
		Name: "echo", Description: "Echo a JSON object", InputSchema: json.RawMessage(`{"type":"object"}`),
	}}}, nil
}

func (echo) CallTool(_ context.Context, call plugin.CallToolParams) (plugin.ToolResult, error) {
	return plugin.Text(string(call.Arguments)), nil
}

func (echo) CallService(_ context.Context, call plugin.ServiceCallParams) (json.RawMessage, error) {
	return call.Arguments, nil
}

func main() {
	if err := plugin.Serve(context.Background(), echo{}); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
