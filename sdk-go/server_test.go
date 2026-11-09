package noraplugin

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"strings"
	"testing"
	"time"
)

type testPlugin struct {
	call func(context.Context, CallToolParams) (ToolResult, error)
}

type servicePlugin struct {
	testPlugin
	services []string
	handler  func(context.Context, ServiceCallParams) (json.RawMessage, error)
}

func (plugin servicePlugin) Initialize(ctx context.Context, params InitializeParams) (InitializeResult, error) {
	result, err := plugin.testPlugin.Initialize(ctx, params)
	result.Services = plugin.services
	return result, err
}

func (plugin servicePlugin) CallService(ctx context.Context, params ServiceCallParams) (json.RawMessage, error) {
	return plugin.handler(ctx, params)
}

func (testPlugin) Initialize(context.Context, InitializeParams) (InitializeResult, error) {
	return InitializeResult{ProtocolVersion: ProtocolVersion, Tools: []ToolDefinition{{
		Name: "echo", Description: "Echo JSON", InputSchema: json.RawMessage(`{"type":"object"}`),
	}}}, nil
}

func (plugin testPlugin) CallTool(ctx context.Context, params CallToolParams) (ToolResult, error) {
	if plugin.call != nil {
		return plugin.call(ctx, params)
	}
	return Text(string(params.Arguments)), nil
}

type session struct {
	input  *io.PipeWriter
	output *io.PipeReader
	reader *json.Decoder
	done   chan error
}

func newSession(t *testing.T, plugin Plugin, options Options) *session {
	t.Helper()
	input, writer := io.Pipe()
	reader, output := io.Pipe()
	ctx, cancel := context.WithCancel(context.Background())
	session := &session{writer, reader, json.NewDecoder(reader), make(chan error, 1)}
	go func() {
		err := ServeIO(ctx, input, output, plugin, options)
		output.Close()
		session.done <- err
	}()
	t.Cleanup(func() { cancel(); writer.Close(); reader.Close() })
	return session
}

func (session *session) send(t *testing.T, value any) {
	t.Helper()
	if err := json.NewEncoder(session.input).Encode(value); err != nil {
		t.Fatal(err)
	}
}

func (session *session) receive(t *testing.T) map[string]json.RawMessage {
	t.Helper()
	type result struct {
		value map[string]json.RawMessage
		err   error
	}
	resultChannel := make(chan result, 1)
	go func() {
		var value map[string]json.RawMessage
		err := session.reader.Decode(&value)
		resultChannel <- result{value, err}
	}()
	select {
	case result := <-resultChannel:
		if result.err != nil {
			t.Fatal(result.err)
		}
		return result.value
	case <-time.After(3 * time.Second):
		session.output.Close()
		t.Fatal("plugin response timed out")
		return nil
	}
}

func (session *session) initialize(t *testing.T, services ...string) {
	t.Helper()
	session.send(t, map[string]any{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": InitializeParams{
		ProtocolVersion: ProtocolVersion, PluginID: "test", Config: json.RawMessage(`{}`), GrantedServices: services,
	}})
	response := session.receive(t)
	if response["error"] != nil {
		t.Fatalf("initialize: %s", response["error"])
	}
	var initialized InitializeResult
	if err := json.Unmarshal(response["result"], &initialized); err != nil {
		t.Fatal(err)
	}
	if initialized.ProtocolVersion != 1 || len(initialized.Tools) != 1 {
		t.Fatalf("initialize: %+v", initialized)
	}
}

func (session *session) shutdown(t *testing.T) {
	t.Helper()
	session.send(t, map[string]any{"jsonrpc": "2.0", "id": 99, "method": "shutdown"})
	response := session.receive(t)
	if string(response["result"]) != "true" {
		t.Fatalf("shutdown: %v", response)
	}
	select {
	case err := <-session.done:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("plugin shutdown timed out")
	}
}

func TestInitializeCallAndShutdown(t *testing.T) {
	session := newSession(t, testPlugin{}, Options{})
	session.initialize(t)
	session.send(t, map[string]any{"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": map[string]any{"name": "echo", "arguments": map[string]any{"text": "hello"}}})
	response := session.receive(t)
	var result ToolResult
	if err := json.Unmarshal(response["result"], &result); err != nil {
		t.Fatal(err)
	}
	if len(result.Content) != 1 || !strings.Contains(result.Content[0].Text, "hello") {
		t.Fatalf("tool result: %+v", result)
	}
	session.shutdown(t)
}

func TestCancellationReachesHandler(t *testing.T) {
	started := make(chan struct{})
	stopped := make(chan struct{})
	session := newSession(t, testPlugin{call: func(ctx context.Context, _ CallToolParams) (ToolResult, error) {
		close(started)
		<-ctx.Done()
		close(stopped)
		return ToolResult{}, ctx.Err()
	}}, Options{})
	session.initialize(t)
	session.send(t, map[string]any{"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": map[string]any{"name": "echo", "arguments": map[string]any{}}})
	<-started
	session.send(t, map[string]any{"jsonrpc": "2.0", "method": "notifications/cancelled", "params": map[string]any{"requestId": 2}})
	response := session.receive(t)
	var rpcError RPCError
	if err := json.Unmarshal(response["error"], &rpcError); err != nil {
		t.Fatal(err)
	}
	if rpcError.Code != -32800 {
		t.Fatalf("cancellation: %+v", rpcError)
	}
	select {
	case <-stopped:
	case <-time.After(time.Second):
		t.Fatal("handler did not observe cancellation")
	}
	session.shutdown(t)
}

func TestHostCallCarriesParentAndUsesGrantedService(t *testing.T) {
	session := newSession(t, testPlugin{call: func(ctx context.Context, _ CallToolParams) (ToolResult, error) {
		result, err := CallService(ctx, "host.info", "get", map[string]any{})
		return Text(string(result)), err
	}}, Options{})
	session.initialize(t, "host.info")
	session.send(t, map[string]any{"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": map[string]any{"name": "echo", "arguments": map[string]any{}}})
	call := session.receive(t)
	var params serviceCallParams
	if err := json.Unmarshal(call["params"], &params); err != nil {
		t.Fatal(err)
	}
	if params.Service != "host.info" || params.ParentRequestID == nil || *params.ParentRequestID != 2 {
		t.Fatalf("host call: %+v", params)
	}
	var id uint64
	if err := json.Unmarshal(call["id"], &id); err != nil {
		t.Fatal(err)
	}
	session.send(t, map[string]any{"jsonrpc": "2.0", "id": id, "result": map[string]any{"name": "Nora"}})
	response := session.receive(t)
	if !bytes.Contains(response["result"], []byte("Nora")) {
		t.Fatalf("host result: %s", response["result"])
	}
	session.shutdown(t)
}

func TestUngrantedHostCallIsRejectedBeforeSending(t *testing.T) {
	session := newSession(t, testPlugin{call: func(ctx context.Context, _ CallToolParams) (ToolResult, error) {
		_, err := CallService(ctx, "credentials", "read", map[string]any{})
		return ToolResult{}, err
	}}, Options{})
	session.initialize(t)
	session.send(t, map[string]any{"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": map[string]any{"name": "echo", "arguments": map[string]any{}}})
	response := session.receive(t)
	var rpcError RPCError
	if err := json.Unmarshal(response["error"], &rpcError); err != nil {
		t.Fatal(err)
	}
	if rpcError.Code != -32003 {
		t.Fatalf("permission: %+v", rpcError)
	}
	session.shutdown(t)
}

func TestFrameBoundsAndTruncation(t *testing.T) {
	for _, test := range []struct {
		name, input string
		expected    error
	}{
		{"too large", strings.Repeat("x", 65) + "\n", ErrFrameTooLarge},
		{"truncated", `{"jsonrpc":"2.0"}`, io.ErrUnexpectedEOF},
	} {
		t.Run(test.name, func(t *testing.T) {
			err := ServeIO(context.Background(), io.NopCloser(strings.NewReader(test.input)), io.Discard, testPlugin{}, Options{MaxFrameBytes: 64})
			if !errors.Is(err, test.expected) {
				t.Fatalf("got %v, expected %v", err, test.expected)
			}
		})
	}
}

func TestEmptyTextRetainsRequiredField(t *testing.T) {
	data, err := json.Marshal(Text(""))
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(data, []byte(`"text":""`)) {
		t.Fatalf("empty text is missing: %s", data)
	}
}

func TestACompletedHandlerCannotReuseItsHostServiceContext(t *testing.T) {
	escaped := make(chan context.Context, 1)
	session := newSession(t, testPlugin{call: func(ctx context.Context, _ CallToolParams) (ToolResult, error) {
		escaped <- ctx
		return Text("done"), nil
	}}, Options{})
	session.initialize(t, "host.info")
	session.send(t, map[string]any{"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": map[string]any{"name": "echo", "arguments": map[string]any{}}})
	response := session.receive(t)
	if response["error"] != nil {
		t.Fatalf("tool call: %s", response["error"])
	}
	ctx := <-escaped
	select {
	case <-ctx.Done():
	case <-time.After(time.Second):
		t.Fatal("completed handler context stayed live")
	}
	if _, err := CallService(ctx, "host.info", "get", map[string]any{}); !errors.Is(err, context.Canceled) {
		t.Fatalf("stale context: %v", err)
	}
	session.shutdown(t)
}

func TestShutdownNotificationProducesNoReply(t *testing.T) {
	session := newSession(t, testPlugin{}, Options{})
	session.initialize(t)
	session.send(t, map[string]any{"jsonrpc": "2.0", "method": "shutdown"})
	select {
	case err := <-session.done:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("shutdown notification did not stop the server")
	}
	var reply any
	if err := session.reader.Decode(&reply); !errors.Is(err, io.EOF) {
		t.Fatalf("notification received an unexpected reply: %v, %v", reply, err)
	}
}

func TestDetachedContextCannotBorrowAReusedRequestID(t *testing.T) {
	escaped := make(chan context.Context, 1)
	live := make(chan struct{})
	release := make(chan struct{})
	session := newSession(t, testPlugin{call: func(ctx context.Context, params CallToolParams) (ToolResult, error) {
		if string(params.Arguments) == `{"hold":true}` {
			close(live)
			select {
			case <-release:
			case <-ctx.Done():
			}
		} else {
			escaped <- ctx
		}
		return Text("done"), nil
	}}, Options{})
	session.initialize(t, "host.info")
	session.send(t, map[string]any{"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": map[string]any{"name": "echo", "arguments": map[string]any{}}})
	if response := session.receive(t); response["error"] != nil {
		t.Fatalf("first call: %s", response["error"])
	}
	ctx := <-escaped
	select {
	case <-ctx.Done():
	case <-time.After(time.Second):
		t.Fatal("first invocation stayed active")
	}
	session.send(t, map[string]any{"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": map[string]any{"name": "echo", "arguments": map[string]any{"hold": true}}})
	select {
	case <-live:
	case <-time.After(time.Second):
		t.Fatal("second invocation did not start")
	}
	denied := make(chan error, 1)
	go func() {
		_, err := CallService(context.WithoutCancel(ctx), "host.info", "get", map[string]any{})
		denied <- err
	}()
	select {
	case err := <-denied:
		var rpcError *RPCError
		if !errors.As(err, &rpcError) || rpcError.Code != -32003 {
			t.Fatalf("detached stale context: %v", err)
		}
	case <-time.After(time.Second):
		t.Fatal("stale context borrowed the second invocation's grant")
	}
	close(release)
	if response := session.receive(t); response["error"] != nil {
		t.Fatalf("second call: %s", response["error"])
	}
	session.shutdown(t)
}

func TestDeclaredServiceCallsAndUnknownServices(t *testing.T) {
	called := 0
	session := newSession(t, servicePlugin{services: []string{"example.echo"}, handler: func(_ context.Context, params ServiceCallParams) (json.RawMessage, error) {
		called++
		return params.Arguments, nil
	}}, Options{})
	session.initialize(t)
	session.send(t, map[string]any{"jsonrpc": "2.0", "id": 2, "method": "services/call", "params": map[string]any{"service": "example.echo", "method": "echo", "arguments": map[string]any{"hello": "Nora"}}})
	response := session.receive(t)
	if !bytes.Contains(response["result"], []byte("Nora")) {
		t.Fatalf("service result: %v", response)
	}
	session.send(t, map[string]any{"jsonrpc": "2.0", "id": 3, "method": "services/call", "params": map[string]any{"service": "undeclared", "method": "echo", "arguments": nil}})
	response = session.receive(t)
	var rpcError RPCError
	if err := json.Unmarshal(response["error"], &rpcError); err != nil || rpcError.Code != -32601 {
		t.Fatalf("undeclared service: %v", response)
	}
	session.shutdown(t)
	if called != 1 {
		t.Fatalf("service calls: %d", called)
	}
}

func TestServiceCallbackUsesItsActualParentRequestAndPreservesNull(t *testing.T) {
	session := newSession(t, servicePlugin{services: []string{"example.echo"}, handler: func(ctx context.Context, _ ServiceCallParams) (json.RawMessage, error) {
		return CallService(ctx, "host.info", "get", nil)
	}}, Options{})
	session.initialize(t, "host.info")
	session.send(t, map[string]any{"jsonrpc": "2.0", "id": 7, "method": "services/call", "params": map[string]any{"service": "example.echo", "method": "echo", "arguments": nil, "parentRequestId": 9999}})
	request := session.receive(t)
	var params serviceCallParams
	if err := json.Unmarshal(request["params"], &params); err != nil || params.ParentRequestID == nil || *params.ParentRequestID != 7 {
		t.Fatalf("service callback parent: %v", request)
	}
	var id uint64
	if err := json.Unmarshal(request["id"], &id); err != nil {
		t.Fatal(err)
	}
	session.send(t, map[string]any{"jsonrpc": "2.0", "id": id, "result": nil})
	response := session.receive(t)
	if string(response["result"]) != "null" {
		t.Fatalf("null service result: %v", response)
	}
	session.shutdown(t)
}

func TestCancellationCleansOneReverseCallAndIgnoresItsLateReply(t *testing.T) {
	session := newSession(t, testPlugin{call: func(ctx context.Context, _ CallToolParams) (ToolResult, error) {
		result, err := CallService(ctx, "host.info", "get", nil)
		return Text(string(result)), err
	}}, Options{})
	session.initialize(t, "host.info")
	request := func(id uint64) {
		session.send(t, map[string]any{"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": map[string]any{"name": "echo", "arguments": map[string]any{}}})
	}
	request(2)
	first := session.receive(t)
	request(3)
	second := session.receive(t)
	var firstID, secondID uint64
	if err := json.Unmarshal(first["id"], &firstID); err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(second["id"], &secondID); err != nil {
		t.Fatal(err)
	}
	session.send(t, map[string]any{"jsonrpc": "2.0", "method": "notifications/cancelled", "params": map[string]any{"requestId": 2}})
	frames := []map[string]json.RawMessage{session.receive(t), session.receive(t)}
	var cancelled, completed bool
	for _, frame := range frames {
		if string(frame["method"]) == `"notifications/cancelled"` {
			var params struct {
				RequestID uint64 `json:"requestId"`
			}
			if err := json.Unmarshal(frame["params"], &params); err != nil || params.RequestID != firstID {
				t.Fatalf("wrong reverse cancellation: %v", frame)
			}
			cancelled = true
		} else {
			var rpcError RPCError
			if err := json.Unmarshal(frame["error"], &rpcError); err != nil || rpcError.Code != -32800 || string(frame["id"]) != "2" {
				t.Fatalf("wrong request cancellation: %v", frame)
			}
			completed = true
		}
	}
	if !cancelled || !completed {
		t.Fatalf("incomplete cancellation: %v", frames)
	}
	session.send(t, map[string]any{"jsonrpc": "2.0", "id": firstID, "result": map[string]any{"late": true}})
	session.send(t, map[string]any{"jsonrpc": "2.0", "id": secondID, "result": map[string]any{"second": true}})
	response := session.receive(t)
	if string(response["id"]) != "3" || !bytes.Contains(response["result"], []byte("second")) {
		t.Fatalf("unrelated reverse request: %v", response)
	}
	session.shutdown(t)
}
