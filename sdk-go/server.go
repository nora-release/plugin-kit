package noraplugin

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"sync"
	"time"
)

var ErrFrameTooLarge = errors.New("plugin protocol frame is too large")

type Plugin interface {
	Initialize(context.Context, InitializeParams) (InitializeResult, error)
	CallTool(context.Context, CallToolParams) (ToolResult, error)
}

type ServicePlugin interface {
	CallService(context.Context, ServiceCallParams) (json.RawMessage, error)
}

type Options struct {
	MaxFrameBytes   int
	MaxInFlight     int
	ShutdownTimeout time.Duration
	// StreamWindow is, in protocol 2, the credit this plugin gives each
	// stream it reads.
	StreamWindow uint64
}

func (options Options) defaults() Options {
	if options.MaxFrameBytes <= 0 {
		options.MaxFrameBytes = 1024 * 1024
	}
	if options.MaxInFlight <= 0 {
		options.MaxInFlight = 32
	}
	if options.ShutdownTimeout <= 0 {
		options.ShutdownTimeout = time.Second
	}
	if options.StreamWindow == 0 {
		options.StreamWindow = DefaultStreamWindow
	}
	return options
}

func Serve(ctx context.Context, plugin Plugin) error {
	return ServeIO(ctx, os.Stdin, os.Stdout, plugin, Options{})
}

// ServeIO owns input and closes it on shutdown to release a blocked read.
// Handlers must honor context cancellation so the server can stop promptly.
func ServeIO(ctx context.Context, input io.ReadCloser, output io.Writer, plugin Plugin, options Options) error {
	if plugin == nil {
		return errors.New("plugin implementation is required")
	}
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	defer input.Close()
	server := &server{
		plugin: plugin, options: options.defaults(), output: output,
		active: make(map[uint64]*serviceInvocation), pending: make(map[uint64]chan response),
		grants: make(map[string]bool), failures: make(chan error, 1),
		provided: make(map[string]bool),
	}
	server.streams = NewStreamTable(PluginSide, server.options.StreamWindow, server.write)
	frames := make(chan readResult, 1)
	go readFrames(ctx, input, frames, server.options.MaxFrameBytes)
	defer server.stop()
	for {
		select {
		case <-ctx.Done():
			return ctx.Err()
		case err := <-server.failures:
			return err
		case frame := <-frames:
			if frame.err != nil {
				if errors.Is(frame.err, io.EOF) {
					return nil
				}
				return frame.err
			}
			stop, err := server.receive(ctx, frame.data)
			if err != nil || stop {
				return err
			}
		}
	}
}

type server struct {
	plugin   Plugin
	options  Options
	output   io.Writer
	writeMu  sync.Mutex
	mu       sync.Mutex
	active   map[uint64]*serviceInvocation
	pending  map[uint64]chan response
	nextID   uint64
	grants   map[string]bool
	provided map[string]bool
	streams  *StreamTable
	version  uint32
	started  bool
	ready    bool
	closing  bool
	workers  sync.WaitGroup
	failures chan error
}

type readResult struct {
	data []byte
	err  error
}

func readFrames(ctx context.Context, input io.Reader, frames chan<- readResult, limit int) {
	reader := bufio.NewReader(input)
	for {
		frame, err := readFrame(reader, limit)
		select {
		case frames <- readResult{frame, err}:
		case <-ctx.Done():
			return
		}
		if err != nil {
			return
		}
	}
}

func readFrame(reader *bufio.Reader, limit int) ([]byte, error) {
	var frame []byte
	for {
		part, err := reader.ReadSlice('\n')
		if len(frame)+len(part) > limit+1 {
			return nil, ErrFrameTooLarge
		}
		frame = append(frame, part...)
		if err == nil {
			return bytes.TrimSuffix(frame, []byte{'\n'}), nil
		}
		if errors.Is(err, bufio.ErrBufferFull) {
			continue
		}
		if errors.Is(err, io.EOF) && len(frame) > 0 {
			return nil, io.ErrUnexpectedEOF
		}
		return nil, err
	}
}

func decodeStrict(data []byte, destination any) error {
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(destination); err != nil {
		return err
	}
	if err := decoder.Decode(new(any)); !errors.Is(err, io.EOF) {
		return errors.New("expected one JSON value")
	}
	return nil
}

func (server *server) receive(ctx context.Context, data []byte) (bool, error) {
	var object map[string]json.RawMessage
	if err := json.Unmarshal(data, &object); err != nil || object == nil {
		return false, server.reply(nil, nil, &RPCError{Code: -32700, Message: "Invalid JSON-RPC frame"})
	}
	if _, ok := object["method"]; !ok {
		return false, server.receiveResponse(data)
	}
	var request request
	if err := decodeStrict(data, &request); err != nil || request.JSONRPC != "2.0" {
		return false, server.reply(request.ID, nil, &RPCError{Code: -32600, Message: "Invalid JSON-RPC request"})
	}
	if request.Method == methodStreamData || request.Method == methodStreamCredit {
		server.mu.Lock()
		version := server.version
		server.mu.Unlock()
		if version != StreamsProtocolVersion {
			return false, &StreamViolation{"a stream frame outside protocol 2"}
		}
		if request.Method == methodStreamData {
			return false, server.streams.OnData(request.Params)
		}
		return false, server.streams.OnCredit(request.Params)
	}
	if request.Method == "notifications/cancelled" {
		var params struct {
			RequestID uint64 `json:"requestId"`
		}
		if err := decodeStrict(request.Params, &params); err != nil {
			return false, nil
		}
		// a reply already sent may still have its output stream running
		server.streams.CloseCall(CallKey{ID: params.RequestID},
			&RPCError{Code: -32800, Message: "The plugin request was cancelled"})
		server.mu.Lock()
		invocation := server.active[params.RequestID]
		server.mu.Unlock()
		if invocation != nil {
			invocation.cancel()
		}
		return false, nil
	}
	if request.Method == "shutdown" {
		if request.ID == nil {
			return true, nil
		}
		return true, server.reply(request.ID, true, nil)
	}
	if request.ID == nil {
		return false, nil
	}
	server.mu.Lock()
	if _, duplicate := server.active[*request.ID]; duplicate {
		server.mu.Unlock()
		return false, errors.New("duplicate in-flight request ID")
	}
	if len(server.active) >= server.options.MaxInFlight {
		server.mu.Unlock()
		return false, server.reply(request.ID, nil, &RPCError{Code: -32001, Message: "Too many in-flight requests"})
	}
	if request.Method == "initialize" {
		if server.started {
			server.mu.Unlock()
			return false, server.reply(request.ID, nil, &RPCError{Code: -32600, Message: "Plugin is already initialized"})
		}
		server.started = true
	} else if !server.ready {
		server.mu.Unlock()
		return false, server.reply(request.ID, nil, &RPCError{Code: -32000, Message: "Initialize the plugin first"})
	}
	requestContext, cancel := context.WithCancel(ctx)
	invocation := &serviceInvocation{cancel: cancel}
	server.active[*request.ID] = invocation
	server.workers.Add(1)
	server.mu.Unlock()
	go server.run(requestContext, invocation, request)
	return false, nil
}

func (server *server) run(ctx context.Context, invocation *serviceInvocation, request request) {
	defer server.workers.Done()
	defer invocation.cancel()
	defer func() {
		server.mu.Lock()
		delete(server.active, *request.ID)
		server.mu.Unlock()
	}()
	var result any
	var err error
	func() {
		defer func() {
			if recover() != nil {
				err = errors.New("plugin handler panicked")
			}
		}()
		ctx = context.WithValue(ctx, serviceContextKey{}, serviceContext{server, *request.ID, invocation})
		result, err = server.invoke(ctx, *request.ID, request)
	}()
	server.mu.Lock()
	invocation.complete = true
	closing := server.closing
	server.mu.Unlock()
	if closing {
		return
	}
	if writeErr := server.reply(request.ID, result, rpcError(err)); writeErr != nil {
		select {
		case server.failures <- writeErr:
		default:
		}
	}
}

func (server *server) invoke(ctx context.Context, id uint64, request request) (any, error) {
	switch request.Method {
	case "initialize":
		var params InitializeParams
		if err := decodeStrict(request.Params, &params); err != nil || params.PluginID == "" ||
			(params.ProtocolVersion != ProtocolVersion && params.ProtocolVersion != StreamsProtocolVersion) {
			return nil, &RPCError{Code: -32602, Message: "Invalid plugin initialization"}
		}
		negotiated := params.ProtocolVersion
		if negotiated == StreamsProtocolVersion {
			if err := server.streams.SetWriteWindow(params.StreamWindow); err != nil {
				return nil, &RPCError{Code: -32602, Message: err.Error()}
			}
		} else if params.StreamWindow != nil {
			return nil, &RPCError{Code: -32602, Message: "streamWindow needs protocol 2"}
		}
		server.mu.Lock()
		for _, service := range params.GrantedServices {
			server.grants[service] = true
		}
		server.mu.Unlock()
		result, err := server.plugin.Initialize(ctx, params)
		if err != nil {
			return nil, err
		}
		// the version is the host's to choose; the plugin speaks it
		result.ProtocolVersion = negotiated
		result.StreamWindow = nil
		if negotiated == StreamsProtocolVersion {
			window := server.streams.ReadWindow()
			result.StreamWindow = &window
		}
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		if result.Tools == nil {
			result.Tools = []ToolDefinition{}
		}
		if result.Services == nil {
			result.Services = []string{}
		}
		if len(result.Services) > 0 {
			if _, ok := server.plugin.(ServicePlugin); !ok {
				return nil, &RPCError{Code: -32602, Message: "Declared services need a service handler"}
			}
		}
		provided := make(map[string]bool, len(result.Services))
		for _, service := range result.Services {
			if service == "" || len(service) > 255 || provided[service] {
				return nil, &RPCError{Code: -32602, Message: "Invalid or duplicate service name"}
			}
			provided[service] = true
		}
		server.mu.Lock()
		server.provided = provided
		server.version = negotiated
		server.ready = true
		server.mu.Unlock()
		return result, nil
	case "tools/call":
		var params callToolParams
		if err := decodeStrict(request.Params, &params); err != nil || params.Name == "" || len(bytes.TrimSpace(params.Arguments)) == 0 || bytes.TrimSpace(params.Arguments)[0] != '{' {
			return nil, &RPCError{Code: -32602, Message: "Invalid tool call parameters"}
		}
		ctx, err := server.attach(ctx, id, params.InputStream, params.OutputStream)
		if err != nil {
			return nil, err
		}
		result, err := server.plugin.CallTool(ctx, CallToolParams{params.Name, params.Arguments})
		if result.Content == nil {
			result.Content = []ContentBlock{}
		}
		return result, err
	case "services/call":
		var params serviceCallParams
		if err := decodeStrict(request.Params, &params); err != nil || params.Service == "" || params.Method == "" {
			return nil, &RPCError{Code: -32602, Message: "Invalid service call parameters"}
		}
		server.mu.Lock()
		declared := server.provided[params.Service]
		server.mu.Unlock()
		plugin, supported := server.plugin.(ServicePlugin)
		if !declared || !supported {
			return nil, &RPCError{Code: -32601, Message: "The plugin service is not supported"}
		}
		ctx, err := server.attach(ctx, id, params.InputStream, params.OutputStream)
		if err != nil {
			return nil, err
		}
		// The inbound ID in ctx is the parent authority. A supplied wire
		// parentRequestId must never replace it in a nested host call.
		return plugin.CallService(ctx, ServiceCallParams{params.Service, params.Method, params.Arguments})
	default:
		return nil, &RPCError{Code: -32601, Message: "The plugin method is not supported"}
	}
}

// attach hands the streams a call names to its handler's context.
func (server *server) attach(ctx context.Context, id uint64, input, output *uint64) (context.Context, error) {
	if input == nil && output == nil {
		return ctx, nil
	}
	server.mu.Lock()
	version := server.version
	server.mu.Unlock()
	if version != StreamsProtocolVersion {
		return ctx, &RPCError{Code: -32602, Message: "Streams need protocol 2"}
	}
	reader, writer, err := server.streams.Accept(CallKey{ID: id}, input, output)
	if err != nil {
		return ctx, err
	}
	return context.WithValue(ctx, streamsContextKey{}, &callStreams{input: reader, output: writer}), nil
}

type streamsContextKey struct{}

type callStreams struct {
	mu     sync.Mutex
	input  *StreamReader
	output *StreamWriter
}

// InputStream is, in protocol 2, the stream the host writes for this call;
// nil when it named none or it was taken already.
func InputStream(ctx context.Context) *StreamReader {
	streams, _ := ctx.Value(streamsContextKey{}).(*callStreams)
	if streams == nil {
		return nil
	}
	streams.mu.Lock()
	defer streams.mu.Unlock()
	reader := streams.input
	streams.input = nil
	return reader
}

// OutputStream is, in protocol 2, the stream this call's answer is written
// to; nil when the host named none or it was taken already. It may outlive
// the handler: return the reply (say an HTTP status and headers) and keep
// writing from a goroutine, which must Close or Abort it in the end.
func OutputStream(ctx context.Context) *StreamWriter {
	streams, _ := ctx.Value(streamsContextKey{}).(*callStreams)
	if streams == nil {
		return nil
	}
	streams.mu.Lock()
	defer streams.mu.Unlock()
	writer := streams.output
	streams.output = nil
	return writer
}

func rpcError(err error) *RPCError {
	if err == nil {
		return nil
	}
	var rpc *RPCError
	if errors.As(err, &rpc) {
		return rpc
	}
	if errors.Is(err, context.Canceled) || errors.Is(err, context.DeadlineExceeded) {
		return &RPCError{Code: -32800, Message: "The plugin request was cancelled"}
	}
	return &RPCError{Code: -32603, Message: err.Error()}
}

func (server *server) reply(id *uint64, result any, rpcError *RPCError) error {
	response := response{JSONRPC: "2.0", ID: id, Error: rpcError}
	if rpcError == nil {
		data, err := json.Marshal(result)
		if err != nil {
			return err
		}
		response.Result = data
	}
	return server.write(response)
}

func (server *server) write(value any) error {
	frame, err := json.Marshal(value)
	if err != nil {
		return err
	}
	if len(frame) > server.options.MaxFrameBytes {
		return ErrFrameTooLarge
	}
	frame = append(frame, '\n')
	server.writeMu.Lock()
	defer server.writeMu.Unlock()
	for len(frame) > 0 {
		count, err := server.output.Write(frame)
		if err != nil {
			return err
		}
		if count == 0 {
			return io.ErrShortWrite
		}
		frame = frame[count:]
	}
	return nil
}

func (server *server) receiveResponse(frame []byte) error {
	var response response
	if err := decodeStrict(frame, &response); err != nil || response.JSONRPC != "2.0" || response.ID == nil || (response.Result == nil) == (response.Error == nil) {
		return errors.New("invalid JSON-RPC response")
	}
	server.mu.Lock()
	pending := server.pending[*response.ID]
	delete(server.pending, *response.ID)
	issued := *response.ID <= server.nextID
	server.mu.Unlock()
	if pending == nil {
		if issued {
			return nil
		}
		return errors.New("response for an unknown request ID")
	}
	pending <- response
	return nil
}

func (server *server) stop() {
	server.streams.CloseAll(&RPCError{Code: -32000, Message: "The plugin connection is closed"})
	server.mu.Lock()
	server.closing = true
	for _, invocation := range server.active {
		invocation.cancel()
	}
	server.mu.Unlock()
	done := make(chan struct{})
	go func() { server.workers.Wait(); close(done) }()
	select {
	case <-done:
	case <-time.After(server.options.ShutdownTimeout):
	}
}

type serviceContextKey struct{}
type serviceInvocation struct {
	cancel   context.CancelFunc
	complete bool
}
type serviceContext struct {
	server     *server
	parentID   uint64
	invocation *serviceInvocation
}

// CallService uses the host-bound caller and parent request; the plugin cannot
// supply a different identity or manufacture an authorization chain.
func CallService(ctx context.Context, service, method string, arguments any) (json.RawMessage, error) {
	caller, ok := ctx.Value(serviceContextKey{}).(serviceContext)
	if !ok {
		return nil, &RPCError{Code: -32003, Message: "No host service context"}
	}
	return caller.server.callService(ctx, caller, service, method, arguments)
}

func (server *server) callService(ctx context.Context, caller serviceContext, service, method string, arguments any) (json.RawMessage, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	data, err := json.Marshal(arguments)
	if err != nil {
		return nil, err
	}
	server.mu.Lock()
	parentActive := server.active[caller.parentID] == caller.invocation && !caller.invocation.complete
	if server.closing || !parentActive || !server.grants[service] {
		server.mu.Unlock()
		return nil, &RPCError{Code: -32003, Message: "The plugin has no grant for this service"}
	}
	if len(server.pending) >= server.options.MaxInFlight {
		server.mu.Unlock()
		return nil, &RPCError{Code: -32001, Message: "Too many in-flight host calls"}
	}
	server.nextID++
	id := server.nextID
	pending := make(chan response, 1)
	server.pending[id] = pending
	server.mu.Unlock()
	defer func() { server.mu.Lock(); delete(server.pending, id); server.mu.Unlock() }()
	params, err := json.Marshal(serviceCallParams{Service: service, Method: method, Arguments: data, ParentRequestID: &caller.parentID})
	if err != nil {
		return nil, err
	}
	if err := server.write(request{JSONRPC: "2.0", ID: &id, Method: "services/call", Params: params}); err != nil {
		return nil, err
	}
	select {
	case <-ctx.Done():
		_ = server.write(request{JSONRPC: "2.0", Method: "notifications/cancelled", Params: json.RawMessage(fmt.Sprintf(`{"requestId":%d}`, id))})
		return nil, ctx.Err()
	case response := <-pending:
		if response.Error != nil {
			return nil, response.Error
		}
		return response.Result, nil
	}
}
