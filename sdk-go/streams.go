package noraplugin

import (
	"encoding/base64"
	"encoding/json"
	"errors"
	"io"
	"sync"
)

// Protocol 2's streams: flow-controlled bytes beside a call. The rules are
// docs/plugin-system.md §6.1.1; the Rust SDK's streams.rs keeps the same
// ones, rule for rule.

const (
	StreamsProtocolVersion uint32 = 2
	DefaultStreamWindow    uint64 = 1024 * 1024
	MinStreamWindow        uint64 = 64 * 1024
	MaxStreamWindow        uint64 = 64 * 1024 * 1024
	// StreamFrameCost is what each streams/data costs besides its bytes.
	StreamFrameCost uint64 = 1024
	// MaxStreamChunk is the most raw bytes one streams/data carries.
	MaxStreamChunk = 256 * 1024

	methodStreamData   = "streams/data"
	methodStreamCredit = "streams/credit"
)

func streamFrameCost(bytes int) uint64 { return uint64(bytes) + StreamFrameCost }

// Side is the end of the connection a table serves: the host allocates odd
// stream ids, a plugin even ones.
type Side int

const (
	HostSide Side = iota
	PluginSide
)

func (side Side) owns(id uint64) bool {
	return id != 0 && (id%2 == 1) == (side == HostSide)
}

// CallKey names the call a stream belongs to: one this side sent (Local) or
// one the other side sent. The two sides' request ids are separate.
type CallKey struct {
	Local bool
	ID    uint64
}

// StreamViolation is a frame the other side should not have sent; the
// connection fails.
type StreamViolation struct{ Reason string }

func (violation *StreamViolation) Error() string {
	return "invalid stream frame: " + violation.Reason
}

type streamData struct {
	Stream uint64    `json:"stream"`
	Data   string    `json:"data,omitempty"`
	End    bool      `json:"end,omitempty"`
	Error  *RPCError `json:"error,omitempty"`
}

type streamCredit struct {
	Stream uint64 `json:"stream"`
	Bytes  uint64 `json:"bytes"`
}

// StreamTable keeps one connection's streams. send writes one frame to the
// connection; the table never calls it with its lock held.
type StreamTable struct {
	side       Side
	readWindow uint64
	send       func(frame any) error

	mu          sync.Mutex
	writeWindow uint64
	nextLocal   uint64
	maxRemote   uint64
	readers     map[uint64]*readerSlot
	writers     map[uint64]*writerSlot
	calls       map[CallKey]map[uint64]struct{}
	closed      *RPCError
}

type readerSlot struct {
	call    CallKey
	chunks  chan []byte
	done    chan struct{}
	err     *RPCError
	granted uint64
	ended   bool
}

type writerSlot struct {
	call   CallKey
	credit uint64
	wake   chan struct{}
	closed *RPCError
}

// NewStreamTable makes a table; readWindow is the credit this side gives
// each stream it reads.
func NewStreamTable(side Side, readWindow uint64, send func(frame any) error) *StreamTable {
	readWindow = min(max(readWindow, MinStreamWindow), MaxStreamWindow)
	first := uint64(2)
	if side == HostSide {
		first = 1
	}
	return &StreamTable{
		side: side, readWindow: readWindow, send: send,
		writeWindow: DefaultStreamWindow, nextLocal: first,
		readers: map[uint64]*readerSlot{}, writers: map[uint64]*writerSlot{},
		calls: map[CallKey]map[uint64]struct{}{},
	}
}

func (table *StreamTable) ReadWindow() uint64 { return table.readWindow }

// SetWriteWindow takes the other side's streamWindow from initialize.
func (table *StreamTable) SetWriteWindow(window *uint64) error {
	value := DefaultStreamWindow
	if window != nil {
		value = *window
	}
	if value < MinStreamWindow || value > MaxStreamWindow {
		return &StreamViolation{"streamWindow is out of range"}
	}
	table.mu.Lock()
	table.writeWindow = value
	table.mu.Unlock()
	return nil
}

// OpenReader is a stream this side reads, for a call it sends.
func (table *StreamTable) OpenReader(call CallKey) (*StreamReader, error) {
	table.mu.Lock()
	defer table.mu.Unlock()
	id, err := table.allocate()
	if err != nil {
		return nil, err
	}
	return table.registerReader(call, id), nil
}

// OpenWriter is a stream this side writes, for a call it sends.
func (table *StreamTable) OpenWriter(call CallKey) (*StreamWriter, error) {
	table.mu.Lock()
	defer table.mu.Unlock()
	id, err := table.allocate()
	if err != nil {
		return nil, err
	}
	return table.registerWriter(call, id), nil
}

// Accept takes the streams a call from the other side names: input this
// side reads, output it writes. Ids must be the other side's and new.
func (table *StreamTable) Accept(call CallKey, input, output *uint64) (*StreamReader, *StreamWriter, error) {
	table.mu.Lock()
	defer table.mu.Unlock()
	if table.closed != nil {
		return nil, nil, table.closed
	}
	var ids []uint64
	for _, id := range []*uint64{input, output} {
		if id != nil {
			ids = append(ids, *id)
		}
	}
	if len(ids) == 2 && ids[0] == ids[1] {
		return nil, nil, &RPCError{Code: -32602, Message: "Invalid or reused stream id"}
	}
	highest := table.maxRemote
	for _, id := range ids {
		if table.side.owns(id) || id <= table.maxRemote {
			return nil, nil, &RPCError{Code: -32602, Message: "Invalid or reused stream id"}
		}
		highest = max(highest, id)
	}
	table.maxRemote = highest
	var reader *StreamReader
	var writer *StreamWriter
	if input != nil {
		reader = table.registerReader(call, *input)
	}
	if output != nil {
		writer = table.registerWriter(call, *output)
	}
	return reader, writer, nil
}

func (table *StreamTable) allocate() (uint64, error) {
	if table.closed != nil {
		return 0, table.closed
	}
	id := table.nextLocal
	if id > ^uint64(0)-2 {
		return 0, &RPCError{Code: -32000, Message: "Stream ids are exhausted"}
	}
	table.nextLocal = id + 2
	return id, nil
}

func (table *StreamTable) track(call CallKey, id uint64) {
	ids := table.calls[call]
	if ids == nil {
		ids = map[uint64]struct{}{}
		table.calls[call] = ids
	}
	ids[id] = struct{}{}
}

func (table *StreamTable) untrack(call CallKey, id uint64) {
	if ids := table.calls[call]; ids != nil {
		delete(ids, id)
		if len(ids) == 0 {
			delete(table.calls, call)
		}
	}
}

func (table *StreamTable) registerReader(call CallKey, id uint64) *StreamReader {
	// every queued frame cost at least StreamFrameCost of the window, so
	// the queue never holds more than this
	slot := &readerSlot{
		call: call, granted: table.readWindow,
		chunks: make(chan []byte, table.readWindow/StreamFrameCost+1),
		done:   make(chan struct{}),
	}
	table.readers[id] = slot
	table.track(call, id)
	return &StreamReader{table: table, id: id, slot: slot}
}

func (table *StreamTable) registerWriter(call CallKey, id uint64) *StreamWriter {
	slot := &writerSlot{call: call, credit: table.writeWindow, wake: make(chan struct{}, 1)}
	table.writers[id] = slot
	table.track(call, id)
	return &StreamWriter{table: table, id: id, slot: slot}
}

// known says whether id was ever a stream of this connection.
func (table *StreamTable) known(id uint64) bool {
	if table.side.owns(id) {
		return id < table.nextLocal
	}
	return id != 0 && id <= table.maxRemote
}

// OnData handles a streams/data from the other side.
func (table *StreamTable) OnData(params json.RawMessage) error {
	var data streamData
	if err := decodeStrict(params, &data); err != nil {
		return &StreamViolation{"malformed streams/data"}
	}
	if data.Error != nil && !data.End {
		return &StreamViolation{"an error without end"}
	}
	bytes, err := base64.StdEncoding.DecodeString(data.Data)
	if err != nil {
		return &StreamViolation{"data is not base64"}
	}
	table.mu.Lock()
	defer table.mu.Unlock()
	if table.closed != nil {
		return nil
	}
	if _, writes := table.writers[data.Stream]; writes {
		return &StreamViolation{"data from the stream's reader"}
	}
	slot := table.readers[data.Stream]
	if slot == nil {
		if table.known(data.Stream) {
			return nil
		}
		return &StreamViolation{"data for an unknown stream"}
	}
	// the closing frame is free: there is one per stream
	cost := uint64(0)
	if !data.End || len(bytes) > 0 {
		cost = streamFrameCost(len(bytes))
	}
	if cost > slot.granted {
		return &StreamViolation{"data beyond the reader's credit"}
	}
	slot.granted -= cost
	if len(bytes) > 0 {
		slot.chunks <- bytes
	}
	if data.End {
		table.endReader(data.Stream, slot, data.Error)
	}
	return nil
}

// OnCredit handles a streams/credit from the other side.
func (table *StreamTable) OnCredit(params json.RawMessage) error {
	var credit streamCredit
	if err := decodeStrict(params, &credit); err != nil {
		return &StreamViolation{"malformed streams/credit"}
	}
	table.mu.Lock()
	defer table.mu.Unlock()
	if table.closed != nil {
		return nil
	}
	if _, reads := table.readers[credit.Stream]; reads {
		return &StreamViolation{"credit from the stream's writer"}
	}
	slot := table.writers[credit.Stream]
	if slot == nil {
		if table.known(credit.Stream) {
			return nil
		}
		return &StreamViolation{"credit for an unknown stream"}
	}
	slot.credit += credit.Bytes
	slot.signal()
	return nil
}

// endReader marks a reader's stream ended, with err when it failed. The
// lock is held.
func (table *StreamTable) endReader(id uint64, slot *readerSlot, err *RPCError) {
	if !slot.ended {
		slot.ended = true
		slot.err = err
		close(slot.done)
	}
	delete(table.readers, id)
	table.untrack(slot.call, id)
}

// closeStream ends one stream with err. The lock is held.
func (table *StreamTable) closeStream(id uint64, err *RPCError) {
	if slot := table.readers[id]; slot != nil {
		table.endReader(id, slot, err)
		return
	}
	if slot := table.writers[id]; slot != nil {
		// kept until its writer goes, so it reads why it was closed
		slot.closed = err
		slot.signal()
		table.untrack(slot.call, id)
	}
}

// CloseCall ends every stream of call: its readers read err, its writers
// stop. Frames of these streams still on the wire are ignored.
func (table *StreamTable) CloseCall(call CallKey, err *RPCError) {
	table.mu.Lock()
	defer table.mu.Unlock()
	for id := range table.calls[call] {
		table.closeStream(id, err)
	}
	delete(table.calls, call)
}

// CloseWriters stops this side's writers of call, which the other side
// answered.
func (table *StreamTable) CloseWriters(call CallKey, err *RPCError) {
	table.mu.Lock()
	defer table.mu.Unlock()
	for id := range table.calls[call] {
		if table.writers[id] != nil {
			table.closeStream(id, err)
		}
	}
}

// HasOpenStreams says whether call has a stream that hasn't ended.
func (table *StreamTable) HasOpenStreams(call CallKey) bool {
	table.mu.Lock()
	defer table.mu.Unlock()
	return len(table.calls[call]) > 0
}

// CloseAll ends every stream: the connection ended.
func (table *StreamTable) CloseAll(err *RPCError) {
	table.mu.Lock()
	defer table.mu.Unlock()
	for id := range table.readers {
		table.closeStream(id, err)
	}
	for id := range table.writers {
		table.closeStream(id, err)
	}
	table.calls = map[CallKey]map[uint64]struct{}{}
	table.closed = err
}

func (slot *writerSlot) signal() {
	select {
	case slot.wake <- struct{}{}:
	default:
	}
}

// StreamReader reads one stream; credit goes back as bytes are taken. It
// is an io.Reader whose Read returns io.EOF once the writer ended the
// stream, or the stream's error. One goroutine reads it.
type StreamReader struct {
	table   *StreamTable
	id      uint64
	slot    *readerSlot
	pending []byte
	spent   uint64
	failed  error
}

func (reader *StreamReader) ID() uint64 { return reader.id }

// Next is the next chunk as the writer sent it, io.EOF at the end.
func (reader *StreamReader) Next() ([]byte, error) {
	if reader.failed != nil {
		return nil, reader.failed
	}
	var chunk []byte
	select {
	case chunk = <-reader.slot.chunks:
	default:
		select {
		case chunk = <-reader.slot.chunks:
		case <-reader.slot.done:
			// what came before the end is read first
			select {
			case chunk = <-reader.slot.chunks:
			default:
				reader.failed = io.EOF
				if reader.slot.err != nil {
					reader.failed = reader.slot.err
				}
				return nil, reader.failed
			}
		}
	}
	reader.spent += streamFrameCost(len(chunk))
	if reader.spent >= reader.table.readWindow/2 {
		if err := reader.table.grant(reader.id, reader.spent); err != nil {
			reader.failed = err
			return nil, err
		}
		reader.spent = 0
	}
	return chunk, nil
}

func (reader *StreamReader) Read(p []byte) (int, error) {
	if len(reader.pending) == 0 {
		chunk, err := reader.Next()
		if err != nil {
			return 0, err
		}
		reader.pending = chunk
	}
	count := copy(p, reader.pending)
	reader.pending = reader.pending[count:]
	return count, nil
}

// Close stops reading: the stream is forgotten, and what is still on the
// wire for it is ignored.
func (reader *StreamReader) Close() error {
	table := reader.table
	table.mu.Lock()
	defer table.mu.Unlock()
	if slot := table.readers[reader.id]; slot == reader.slot {
		delete(table.readers, reader.id)
		table.untrack(slot.call, reader.id)
	}
	if reader.failed == nil {
		reader.failed = io.ErrClosedPipe
	}
	return nil
}

// grant gives the writer more credit. Never dropped: a lost credit would
// leave the writer waiting for good.
func (table *StreamTable) grant(id, bytes uint64) error {
	table.mu.Lock()
	slot := table.readers[id]
	if slot != nil {
		slot.granted += bytes
	}
	table.mu.Unlock()
	if slot == nil {
		return nil
	}
	return table.send(notification(methodStreamCredit, streamCredit{id, bytes}))
}

// StreamWriter writes one stream, waiting for credit. It is an io.Writer;
// Close ends the stream and Abort ends it with an error. One goroutine
// writes it.
type StreamWriter struct {
	table *StreamTable
	id    uint64
	slot  *writerSlot
	done  bool
}

func (writer *StreamWriter) ID() uint64 { return writer.id }

// Write sends p in as many frames as the reader's credit allows. A frame
// takes what the credit has room for rather than waiting for a whole
// chunk: the reader returns credit only once half its window is spent.
func (writer *StreamWriter) Write(p []byte) (int, error) {
	if writer.done {
		return 0, errors.New("the stream has ended")
	}
	chunk := writer.table.chunkSize()
	written := 0
	for written < len(p) {
		count, err := writer.reserve(min(len(p)-written, chunk))
		if err != nil {
			return written, err
		}
		if err := writer.sendData(p[written:written+count], false, nil); err != nil {
			return written, err
		}
		written += count
	}
	return written, nil
}

// Close ends the stream.
func (writer *StreamWriter) Close() error { return writer.end(nil) }

// Abort ends the stream with an error the reader reads.
func (writer *StreamWriter) Abort(err *RPCError) error {
	if err == nil {
		err = &RPCError{Code: -32000, Message: "The stream writer stopped"}
	}
	return writer.end(err)
}

func (writer *StreamWriter) end(err *RPCError) error {
	if writer.done {
		return nil
	}
	writer.done = true
	table := writer.table
	table.mu.Lock()
	closed := writer.slot.closed
	if table.writers[writer.id] == writer.slot {
		delete(table.writers, writer.id)
		table.untrack(writer.slot.call, writer.id)
	}
	table.mu.Unlock()
	if closed != nil {
		return closed
	}
	return writer.sendData(nil, true, err)
}

// reserve takes credit for up to wanted bytes, at least one, waiting while
// there is no room for a frame.
func (writer *StreamWriter) reserve(wanted int) (int, error) {
	table := writer.table
	for {
		table.mu.Lock()
		if writer.slot.closed != nil {
			err := writer.slot.closed
			table.mu.Unlock()
			return 0, err
		}
		if writer.slot.credit > StreamFrameCost {
			count := min(uint64(wanted), writer.slot.credit-StreamFrameCost)
			writer.slot.credit -= streamFrameCost(int(count))
			table.mu.Unlock()
			return int(count), nil
		}
		table.mu.Unlock()
		<-writer.slot.wake
	}
}

func (writer *StreamWriter) sendData(bytes []byte, end bool, err *RPCError) error {
	return writer.table.send(notification(methodStreamData, streamData{
		Stream: writer.id, Data: base64.StdEncoding.EncodeToString(bytes), End: end, Error: err,
	}))
}

// chunkSize is the largest raw chunk the other side's window takes in one
// frame.
func (table *StreamTable) chunkSize() int {
	table.mu.Lock()
	window := table.writeWindow
	table.mu.Unlock()
	return int(min(max(window-StreamFrameCost, 1), MaxStreamChunk))
}

func notification(method string, params any) any {
	data, _ := json.Marshal(params)
	return request{JSONRPC: "2.0", Method: method, Params: data}
}
