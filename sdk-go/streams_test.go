package noraplugin

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"sync"
	"testing"
	"time"
)

// pair is a host table and a plugin table, each one's frames handed to the
// other in order by a goroutine. Violations are collected.
type pair struct {
	host, plugin *StreamTable
	mu           sync.Mutex
	violations   []error
}

func newPair(t *testing.T, window uint64) *pair {
	t.Helper()
	p := &pair{}
	toPlugin := make(chan any, 4096)
	toHost := make(chan any, 4096)
	p.host = NewStreamTable(HostSide, window, func(frame any) error { toPlugin <- frame; return nil })
	p.plugin = NewStreamTable(PluginSide, window, func(frame any) error { toHost <- frame; return nil })
	hostWindow, pluginWindow := p.host.ReadWindow(), p.plugin.ReadWindow()
	if err := p.host.SetWriteWindow(&pluginWindow); err != nil {
		t.Fatal(err)
	}
	if err := p.plugin.SetWriteWindow(&hostWindow); err != nil {
		t.Fatal(err)
	}
	done := make(chan struct{})
	t.Cleanup(func() { close(done) })
	go p.pump(toPlugin, p.plugin, done)
	go p.pump(toHost, p.host, done)
	return p
}

func (p *pair) pump(frames chan any, to *StreamTable, done chan struct{}) {
	for {
		select {
		case <-done:
			return
		case frame := <-frames:
			message := frame.(request)
			var err error
			if message.Method == methodStreamData {
				err = to.OnData(message.Params)
			} else {
				err = to.OnCredit(message.Params)
			}
			if err != nil {
				p.mu.Lock()
				p.violations = append(p.violations, err)
				p.mu.Unlock()
			}
		}
	}
}

func (p *pair) clean(t *testing.T) {
	t.Helper()
	p.mu.Lock()
	defer p.mu.Unlock()
	if len(p.violations) > 0 {
		t.Fatalf("violations: %v", p.violations)
	}
}

var (
	call     = CallKey{Local: true, ID: 7}
	accepted = CallKey{Local: false, ID: 7}
)

func readAll(t *testing.T, reader io.Reader) ([]byte, error) {
	t.Helper()
	type result struct {
		data []byte
		err  error
	}
	results := make(chan result, 1)
	go func() { data, err := io.ReadAll(reader); results <- result{data, err} }()
	select {
	case result := <-results:
		return result.data, result.err
	case <-time.After(10 * time.Second):
		t.Fatal("the stream stalled")
		return nil, nil
	}
}

func ptr(value uint64) *uint64 { return &value }

func TestAStreamLargerThanTheWindowArrivesWholeAndInOrder(t *testing.T) {
	p := newPair(t, MinStreamWindow)
	writer, _ := p.host.OpenWriter(call)
	reader, _, err := p.plugin.Accept(accepted, ptr(writer.ID()), nil)
	if err != nil {
		t.Fatal(err)
	}
	sent := make([]byte, 1_000_000)
	for i := range sent {
		sent[i] = byte(i % 251)
	}
	go func() {
		// a short frame first: the writer then has less credit than a chunk
		_, _ = writer.Write(sent[:4096])
		_, _ = writer.Write(sent[4096:])
		_ = writer.Close()
	}()
	received, err := readAll(t, reader)
	if err != nil || !bytes.Equal(received, sent) {
		t.Fatalf("received %d bytes, %v", len(received), err)
	}
	p.clean(t)
	if p.host.HasOpenStreams(call) {
		t.Fatal("the finished stream is still open")
	}
}

func TestAWriterWaitsWhileItsReaderTakesNothing(t *testing.T) {
	p := newPair(t, MinStreamWindow)
	writer, _ := p.host.OpenWriter(call)
	if _, _, err := p.plugin.Accept(accepted, ptr(writer.ID()), nil); err != nil {
		t.Fatal(err)
	}
	wrote := make(chan struct{})
	go func() { _, _ = writer.Write(make([]byte, 3*MinStreamWindow)); close(wrote) }()
	select {
	case <-wrote:
		t.Fatal("the writer went past the reader's window")
	case <-time.After(200 * time.Millisecond):
	}
	p.host.CloseCall(call, &RPCError{Code: -32800, Message: "cancelled"})
	<-wrote
}

func TestACancelledCallEndsItsStreams(t *testing.T) {
	p := newPair(t, DefaultStreamWindow)
	reader, _ := p.host.OpenReader(call)
	_, writer, err := p.plugin.Accept(accepted, nil, ptr(reader.ID()))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := writer.Write([]byte("head")); err != nil {
		t.Fatal(err)
	}
	if chunk, err := reader.Next(); err != nil || string(chunk) != "head" {
		t.Fatalf("%q %v", chunk, err)
	}
	cancelled := &RPCError{Code: -32800, Message: "cancelled"}
	p.host.CloseCall(call, cancelled)
	p.plugin.CloseCall(accepted, cancelled)
	if _, err := reader.Next(); err != cancelled {
		t.Fatalf("reader: %v", err)
	}
	if _, err := writer.Write([]byte("late")); err != cancelled {
		t.Fatalf("writer: %v", err)
	}
	// a frame of the closed stream already on the wire is ignored
	if err := p.host.OnData(json.RawMessage(`{"stream":` + itoa(reader.ID()) + `,"data":"eA=="}`)); err != nil {
		t.Fatal(err)
	}
	p.clean(t)
}

func TestAWriterThatGivesUpTellsItsReader(t *testing.T) {
	p := newPair(t, DefaultStreamWindow)
	reader, _ := p.host.OpenReader(call)
	_, writer, _ := p.plugin.Accept(accepted, nil, ptr(reader.ID()))
	_ = writer.Abort(&RPCError{Code: -32050, Message: "upstream failed"})
	_, err := reader.Next()
	var rpc *RPCError
	if !errors.As(err, &rpc) || rpc.Code != -32050 {
		t.Fatalf("%v", err)
	}
}

func TestStreamIDsAreTheCallersAndNeverReused(t *testing.T) {
	p := newPair(t, DefaultStreamWindow)
	first, _ := p.host.OpenWriter(call)
	if first.ID()%2 != 1 {
		t.Fatalf("host id %d", first.ID())
	}
	if _, _, err := p.plugin.Accept(accepted, ptr(2), nil); err == nil {
		t.Fatal("took a plugin-side id")
	}
	if _, _, err := p.plugin.Accept(accepted, ptr(first.ID()), nil); err != nil {
		t.Fatal(err)
	}
	if _, _, err := p.plugin.Accept(accepted, ptr(first.ID()), nil); err == nil {
		t.Fatal("took a reused id")
	}
	if _, _, err := p.plugin.Accept(accepted, ptr(9), ptr(9)); err == nil {
		t.Fatal("took one id for both streams")
	}
	if reader, _ := p.plugin.OpenReader(call); reader.ID() != 2 {
		t.Fatalf("plugin id %d", reader.ID())
	}
}

func TestFramesAPeerShouldNotSendAreViolations(t *testing.T) {
	p := newPair(t, MinStreamWindow)
	reader, _ := p.host.OpenReader(call)
	writer, _ := p.host.OpenWriter(call)
	over := make([]byte, MinStreamWindow)
	for _, frame := range []struct {
		data bool
		body string
	}{
		{true, `{"stream":99}`},
		{false, `{"stream":99,"bytes":1}`},
		{true, `{"stream":` + itoa(writer.ID()) + `}`},
		{false, `{"stream":` + itoa(reader.ID()) + `,"bytes":1}`},
		{true, `{"stream":` + itoa(reader.ID()) + `,"error":{"code":1,"message":"x"}}`},
		{true, `{"stream":` + itoa(reader.ID()) + `,"data":"!"}`},
		{true, `{"stream":` + itoa(reader.ID()) + `,"data":"` + base64Of(over) + `"}`},
	} {
		var err error
		if frame.data {
			err = p.host.OnData(json.RawMessage(frame.body))
		} else {
			err = p.host.OnCredit(json.RawMessage(frame.body))
		}
		var violation *StreamViolation
		if !errors.As(err, &violation) {
			t.Fatalf("%s: %v", frame.body, err)
		}
	}
}

func TestAClosedConnectionEndsEveryStream(t *testing.T) {
	p := newPair(t, DefaultStreamWindow)
	reader, _ := p.host.OpenReader(call)
	writer, _ := p.host.OpenWriter(call)
	gone := &RPCError{Code: -32000, Message: "gone"}
	p.host.CloseAll(gone)
	if _, err := reader.Next(); err != gone {
		t.Fatalf("reader: %v", err)
	}
	if _, err := writer.Write([]byte("x")); err != gone {
		t.Fatalf("writer: %v", err)
	}
	if _, err := p.host.OpenReader(call); err == nil {
		t.Fatal("opened a stream on a closed connection")
	}
}

// streamPlugin's copy service, as a provider's fetch, reads its whole input,
// answers, then writes it to its output stream; spill writes n bytes and
// ends or gives up.
type streamPlugin struct{ testPlugin }

func (streamPlugin) Initialize(ctx context.Context, params InitializeParams) (InitializeResult, error) {
	return InitializeResult{Services: []string{"example.echo"}}, nil
}

func (streamPlugin) CallService(ctx context.Context, params ServiceCallParams) (json.RawMessage, error) {
	switch params.Method {
	case "copy":
		input, output := InputStream(ctx), OutputStream(ctx)
		if input == nil || output == nil {
			return nil, &RPCError{Code: -32602, Message: "copy needs both streams"}
		}
		body, err := io.ReadAll(input)
		if err != nil {
			return nil, err
		}
		go func() {
			if _, err := output.Write(body); err != nil {
				_ = output.Abort(nil)
				return
			}
			_ = output.Close()
		}()
		return json.RawMessage(`{"head":true}`), nil
	case "spill":
		var arguments struct {
			Bytes int  `json:"bytes"`
			Fail  bool `json:"fail"`
		}
		_ = json.Unmarshal(params.Arguments, &arguments)
		output := OutputStream(ctx)
		if output == nil {
			return nil, &RPCError{Code: -32602, Message: "spill needs an output stream"}
		}
		if _, err := output.Write(bytes.Repeat([]byte("x"), arguments.Bytes)); err != nil {
			return nil, err
		}
		if arguments.Fail {
			_ = output.Abort(&RPCError{Code: -32050, Message: "spill failed"})
		} else {
			_ = output.Close()
		}
		return json.RawMessage(`{"wrote":` + itoa(uint64(arguments.Bytes)) + `}`), nil
	}
	return json.RawMessage(`{}`), nil
}

// streamingHost drives a served plugin as a protocol 2 host: its stream
// frames go to a table, everything else to replies.
type streamingHost struct {
	table   *StreamTable
	input   *io.PipeWriter
	writeMu sync.Mutex
	replies chan map[string]json.RawMessage
	done    chan error
}

func startStreamingHost(t *testing.T, version uint32) *streamingHost {
	t.Helper()
	input, toPlugin := io.Pipe()
	fromPlugin, output := io.Pipe()
	ctx, cancel := context.WithCancel(context.Background())
	host := &streamingHost{input: toPlugin, replies: make(chan map[string]json.RawMessage, 64), done: make(chan error, 1)}
	host.table = NewStreamTable(HostSide, MinStreamWindow, func(frame any) error { return host.send(frame) })
	go func() { host.done <- ServeIO(ctx, input, output, streamPlugin{}, Options{}); output.Close() }()
	go func() {
		lines := bufio.NewReaderSize(fromPlugin, 2*1024*1024)
		for {
			line, err := lines.ReadBytes('\n')
			if err != nil {
				return
			}
			var frame map[string]json.RawMessage
			if json.Unmarshal(line, &frame) != nil {
				return
			}
			var method string
			_ = json.Unmarshal(frame["method"], &method)
			switch method {
			case methodStreamData:
				if err := host.table.OnData(frame["params"]); err != nil {
					panic(err)
				}
			case methodStreamCredit:
				if err := host.table.OnCredit(frame["params"]); err != nil {
					panic(err)
				}
			default:
				host.replies <- frame
			}
		}
	}()
	t.Cleanup(func() { cancel(); toPlugin.Close(); fromPlugin.Close() })
	params := map[string]any{"protocolVersion": version, "pluginId": "test", "config": map[string]any{}, "grantedServices": []string{}}
	if version == StreamsProtocolVersion {
		params["streamWindow"] = host.table.ReadWindow()
	}
	if err := host.send(map[string]any{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": params}); err != nil {
		t.Fatal(err)
	}
	var initialized InitializeResult
	if err := json.Unmarshal(host.reply(t)["result"], &initialized); err != nil {
		t.Fatal(err)
	}
	if initialized.ProtocolVersion != version {
		t.Fatalf("answered %d", initialized.ProtocolVersion)
	}
	if err := host.table.SetWriteWindow(initialized.StreamWindow); err != nil {
		t.Fatal(err)
	}
	return host
}

func (host *streamingHost) send(frame any) error {
	data, err := json.Marshal(frame)
	if err != nil {
		return err
	}
	host.writeMu.Lock()
	defer host.writeMu.Unlock()
	_, err = host.input.Write(append(data, '\n'))
	return err
}

func (host *streamingHost) reply(t *testing.T) map[string]json.RawMessage {
	t.Helper()
	select {
	case reply := <-host.replies:
		return reply
	case <-time.After(5 * time.Second):
		t.Fatal("no reply")
		return nil
	}
}

func (host *streamingHost) call(t *testing.T, id uint64, method string, arguments string, input, output *uint64) {
	t.Helper()
	params := map[string]any{"service": "example.echo", "method": method, "arguments": json.RawMessage(arguments)}
	if input != nil {
		params["inputStream"] = *input
	}
	if output != nil {
		params["outputStream"] = *output
	}
	if err := host.send(map[string]any{"jsonrpc": "2.0", "id": id, "method": "services/call", "params": params}); err != nil {
		t.Fatal(err)
	}
}

func TestProtocolTwoReadsABodyAnswersThenStreamsOneLargerThanAnyFrame(t *testing.T) {
	host := startStreamingHost(t, StreamsProtocolVersion)
	key := CallKey{Local: true, ID: 2}
	input, _ := host.table.OpenWriter(key)
	output, _ := host.table.OpenReader(key)
	host.call(t, 2, "copy", `{}`, ptr(input.ID()), ptr(output.ID()))
	body := make([]byte, 3_000_000)
	for i := range body {
		body[i] = byte(i % 253)
	}
	go func() { _, _ = input.Write(body); _ = input.Close() }()
	// the head comes once the input is in, before any of the body
	if string(host.reply(t)["result"]) != `{"head":true}` {
		t.Fatal("no head")
	}
	host.table.CloseWriters(key, &RPCError{Code: -32000, Message: "answered"})
	copied, err := readAll(t, output)
	if err != nil || !bytes.Equal(copied, body) {
		t.Fatalf("copied %d bytes, %v", len(copied), err)
	}
}

func TestAPluginThatGivesUpMidStreamEndsItWithItsError(t *testing.T) {
	host := startStreamingHost(t, StreamsProtocolVersion)
	output, _ := host.table.OpenReader(CallKey{Local: true, ID: 2})
	host.call(t, 2, "spill", `{"bytes":100000,"fail":true}`, nil, ptr(output.ID()))
	read, err := readAll(t, output)
	var rpc *RPCError
	if len(read) != 100_000 || !errors.As(err, &rpc) || rpc.Code != -32050 {
		t.Fatalf("read %d, %v", len(read), err)
	}
	if string(host.reply(t)["result"]) != `{"wrote":100000}` {
		t.Fatal("no reply after the stream")
	}
}

func TestACancelledStreamingCallLeavesThePluginServing(t *testing.T) {
	host := startStreamingHost(t, StreamsProtocolVersion)
	input, _ := host.table.OpenWriter(CallKey{Local: true, ID: 2})
	output, _ := host.table.OpenReader(CallKey{Local: true, ID: 2})
	host.call(t, 2, "copy", `{}`, ptr(input.ID()), ptr(output.ID()))
	// part of the input, then the call is cancelled mid-way
	if _, err := input.Write(make([]byte, 10_000)); err != nil {
		t.Fatal(err)
	}
	_ = host.send(map[string]any{"jsonrpc": "2.0", "method": "notifications/cancelled", "params": map[string]any{"requestId": 2}})
	host.reply(t)
	host.table.CloseCall(CallKey{Local: true, ID: 2}, &RPCError{Code: -32800, Message: "cancelled"})
	host.call(t, 3, "echo", `{}`, nil, nil)
	if string(host.reply(t)["id"]) != "3" {
		t.Fatal("the plugin stopped serving")
	}
}

func TestProtocolOneTakesNoStreams(t *testing.T) {
	host := startStreamingHost(t, ProtocolVersion)
	host.call(t, 2, "spill", `{}`, nil, ptr(1))
	var failure RPCError
	if err := json.Unmarshal(host.reply(t)["error"], &failure); err != nil || failure.Code != -32602 {
		t.Fatalf("%+v %v", failure, err)
	}
	_ = host.send(map[string]any{"jsonrpc": "2.0", "method": "streams/credit", "params": map[string]any{"stream": 1, "bytes": 1}})
	select {
	case err := <-host.done:
		var violation *StreamViolation
		if !errors.As(err, &violation) {
			t.Fatalf("ended with %v", err)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("a stream frame in protocol 1 did not end the session")
	}
}

func itoa(value uint64) string {
	data, _ := json.Marshal(value)
	return string(data)
}

func base64Of(data []byte) string {
	encoded, _ := json.Marshal(data)
	return string(encoded[1 : len(encoded)-1])
}

// A call's first data may come right behind the call, in the same write:
// its streams are the plugin's as it reads the call, not once its handler
// runs.
func TestAStreamsFirstFrameRightBehindItsCallIsTaken(t *testing.T) {
	for i := 0; i < 50; i++ {
		host := startStreamingHost(t, StreamsProtocolVersion)
		key := CallKey{Local: true, ID: 2}
		input, _ := host.table.OpenWriter(key)
		output, _ := host.table.OpenReader(key)
		call, _ := json.Marshal(map[string]any{"jsonrpc": "2.0", "id": 2, "method": "services/call", "params": map[string]any{
			"service": "example.echo", "method": "copy", "arguments": map[string]any{},
			"inputStream": input.ID(), "outputStream": output.ID()}})
		data, _ := json.Marshal(notification(methodStreamData, streamData{Stream: input.ID(), Data: "aGk=", End: true}))
		host.writeMu.Lock()
		_, err := host.input.Write(append(append(append(call, '\n'), data...), '\n'))
		host.writeMu.Unlock()
		if err != nil {
			t.Fatal(err)
		}
		if reply := host.reply(t); string(reply["result"]) != `{"head":true}` {
			t.Fatalf("round %d: %s", i, reply["error"])
		}
		if got, err := readAll(t, output); err != nil || string(got) != "hi" {
			t.Fatalf("round %d: %q %v", i, got, err)
		}
	}
}
