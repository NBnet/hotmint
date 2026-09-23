package hotmint

import (
	"bytes"
	"context"
	"errors"
	"net"
	"os"
	"path/filepath"
	"testing"
	"time"

	pb "github.com/rust-util-collections/hotmint/sdk/go/proto/abci"
	"google.golang.org/protobuf/proto"
)

// --- Frame protocol tests ---

func TestFrameRoundtrip(t *testing.T) {
	var buf bytes.Buffer
	payload := []byte("hello hotmint")

	if err := WriteFrame(&buf, payload); err != nil {
		t.Fatalf("WriteFrame: %v", err)
	}

	got, err := ReadFrame(&buf)
	if err != nil {
		t.Fatalf("ReadFrame: %v", err)
	}

	if !bytes.Equal(got, payload) {
		t.Fatalf("got %q, want %q", got, payload)
	}
}

func TestFrameEmpty(t *testing.T) {
	var buf bytes.Buffer

	if err := WriteFrame(&buf, nil); err != nil {
		t.Fatalf("WriteFrame: %v", err)
	}

	got, err := ReadFrame(&buf)
	if err != nil {
		t.Fatalf("ReadFrame: %v", err)
	}

	if len(got) != 0 {
		t.Fatalf("expected empty, got %d bytes", len(got))
	}
}

func TestWriteFrameRejectsOversizedPayloadBeforeWriting(t *testing.T) {
	var buf bytes.Buffer
	if err := WriteFrame(&buf, make([]byte, maxFrameSize+1)); err == nil {
		t.Fatal("expected oversized frame error")
	}
	if buf.Len() != 0 {
		t.Fatal("oversized frame must not write a length prefix")
	}
}

type emptyErrorApp struct{ BaseApplication }

func (emptyErrorApp) InitChain([]byte) ([]byte, error) { return nil, errors.New("") }
func (emptyErrorApp) ExecuteBlock([][]byte, *pb.BlockContext) (*pb.EndBlockResponse, error) {
	return nil, errors.New("")
}
func (emptyErrorApp) OnCommit(*pb.Block, *pb.BlockContext) error      { return errors.New("") }
func (emptyErrorApp) OnEvidence(*pb.EquivocationProof) error          { return errors.New("") }
func (emptyErrorApp) OnOfflineValidators([]*pb.OfflineEvidence) error { return errors.New("") }
func (emptyErrorApp) Query(string, []byte) (*QueryResult, error)      { return nil, errors.New("") }

func TestEmptyApplicationErrorsRemainFailuresOnWire(t *testing.T) {
	srv := NewServer("", emptyErrorApp{})
	cases := []struct {
		name         string
		request      *pb.Request
		errorMessage func(*pb.Response) string
	}{
		{"init", &pb.Request{Request: &pb.Request_InitChain{InitChain: &pb.InitChainRequest{}}}, func(r *pb.Response) string { return r.GetInitChain().Error }},
		{"execute", &pb.Request{Request: &pb.Request_ExecuteBlock{ExecuteBlock: &pb.ExecuteBlockRequest{}}}, func(r *pb.Response) string { return r.GetExecuteBlock().Error }},
		{"commit", &pb.Request{Request: &pb.Request_OnCommit{OnCommit: &pb.OnCommitRequest{}}}, func(r *pb.Response) string { return r.GetOnCommit().Error }},
		{"evidence", &pb.Request{Request: &pb.Request_OnEvidence{OnEvidence: &pb.EquivocationProof{}}}, func(r *pb.Response) string { return r.GetOnEvidence().Error }},
		{"offline", &pb.Request{Request: &pb.Request_OnOfflineValidators{OnOfflineValidators: &pb.OnOfflineValidatorsRequest{}}}, func(r *pb.Response) string { return r.GetOnOfflineValidators().Error }},
		{"query", &pb.Request{Request: &pb.Request_Query{Query: &pb.QueryRequest{}}}, func(r *pb.Response) string { return r.GetQuery().Error }},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			data, err := proto.Marshal(srv.dispatch(tc.request))
			if err != nil {
				t.Fatal(err)
			}
			var response pb.Response
			if err := proto.Unmarshal(data, &response); err != nil {
				t.Fatal(err)
			}
			if tc.errorMessage(&response) == "" {
				t.Fatal("application failure encoded as success")
			}
		})
	}
}

// --- testApp for server tests ---

type testApp struct {
	BaseApplication
	commitCount int
}

func (a *testApp) CreatePayload(_ *pb.BlockContext) []byte {
	return []byte("test-payload")
}

func (a *testApp) ExecuteBlock(_ [][]byte, _ *pb.BlockContext) (*pb.EndBlockResponse, error) {
	return NewEndBlockResponse(), nil
}

func (a *testApp) OnCommit(_ *pb.Block, _ *pb.BlockContext) error {
	a.commitCount++
	return nil
}

func (a *testApp) Query(_ string, data []byte) (*QueryResult, error) {
	return &QueryResult{Data: data}, nil // echo
}

// --- Server integration test (Go-to-Go) ---

func TestServerCreatePayload(t *testing.T) {
	sock := tempSocket(t)
	app := &testApp{}
	srv := NewServer(sock, app)

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	go srv.Run(ctx)
	time.Sleep(50 * time.Millisecond) // Wait for server to start.

	conn, err := net.Dial("unix", sock)
	if err != nil {
		t.Fatalf("dial: %v", err)
	}
	defer conn.Close()

	// Send CreatePayload request.
	reqCtx := &pb.BlockContext{Height: 1, View: 1, Proposer: 0, Epoch: 0}
	req := &pb.Request{
		Request: &pb.Request_CreatePayload{CreatePayload: reqCtx},
	}
	sendRequest(t, conn, req)

	// Read response.
	resp := readResponse(t, conn)
	cp := resp.GetCreatePayload()
	if cp == nil {
		t.Fatalf("expected CreatePayload response, got %T", resp.Response)
	}
	if !bytes.Equal(cp.Payload, []byte("test-payload")) {
		t.Fatalf("got payload %q, want %q", cp.Payload, "test-payload")
	}
}

func TestServerValidateBlock(t *testing.T) {
	sock := tempSocket(t)
	srv := NewServer(sock, &testApp{})

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	go srv.Run(ctx)
	time.Sleep(50 * time.Millisecond)

	conn, err := net.Dial("unix", sock)
	if err != nil {
		t.Fatalf("dial: %v", err)
	}
	defer conn.Close()

	req := &pb.Request{
		Request: &pb.Request_ValidateBlock{
			ValidateBlock: &pb.ValidateBlockRequest{
				Block: &pb.Block{Height: 1, View: 1},
				Ctx:   &pb.BlockContext{Height: 1},
			},
		},
	}
	sendRequest(t, conn, req)

	resp := readResponse(t, conn)
	vb := resp.GetValidateBlock()
	if vb == nil {
		t.Fatalf("expected ValidateBlock response, got %T", resp.Response)
	}
	if !vb.Ok {
		t.Fatal("expected ValidateBlock to return true")
	}
}

func TestServerExecuteBlock(t *testing.T) {
	sock := tempSocket(t)
	srv := NewServer(sock, &testApp{})

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	go srv.Run(ctx)
	time.Sleep(50 * time.Millisecond)

	conn, err := net.Dial("unix", sock)
	if err != nil {
		t.Fatalf("dial: %v", err)
	}
	defer conn.Close()

	req := &pb.Request{
		Request: &pb.Request_ExecuteBlock{
			ExecuteBlock: &pb.ExecuteBlockRequest{
				Txs: [][]byte{{1, 2, 3}},
				Ctx: &pb.BlockContext{Height: 1},
			},
		},
	}
	sendRequest(t, conn, req)

	resp := readResponse(t, conn)
	eb := resp.GetExecuteBlock()
	if eb == nil {
		t.Fatalf("expected ExecuteBlock response, got %T", resp.Response)
	}
	if eb.Error != "" {
		t.Fatalf("unexpected error: %s", eb.Error)
	}
}

func TestServerQuery(t *testing.T) {
	sock := tempSocket(t)
	srv := NewServer(sock, &testApp{})

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	go srv.Run(ctx)
	time.Sleep(50 * time.Millisecond)

	conn, err := net.Dial("unix", sock)
	if err != nil {
		t.Fatalf("dial: %v", err)
	}
	defer conn.Close()

	data := []byte("query-data")
	req := &pb.Request{
		Request: &pb.Request_Query{
			Query: &pb.QueryRequest{Path: "/state", Data: data},
		},
	}
	sendRequest(t, conn, req)

	resp := readResponse(t, conn)
	qr := resp.GetQuery()
	if qr == nil {
		t.Fatalf("expected Query response, got %T", resp.Response)
	}
	if qr.Error != "" {
		t.Fatalf("unexpected error: %s", qr.Error)
	}
	if !bytes.Equal(qr.Data, data) {
		t.Fatalf("got %q, want %q", qr.Data, data)
	}
}

func TestServerMultipleRequests(t *testing.T) {
	sock := tempSocket(t)
	srv := NewServer(sock, &testApp{})

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	go srv.Run(ctx)
	time.Sleep(50 * time.Millisecond)

	conn, err := net.Dial("unix", sock)
	if err != nil {
		t.Fatalf("dial: %v", err)
	}
	defer conn.Close()

	// Send 10 requests on the same connection.
	for i := range 10 {
		data := []byte{byte(i)}
		req := &pb.Request{
			Request: &pb.Request_Query{
				Query: &pb.QueryRequest{Path: "/test", Data: data},
			},
		}
		sendRequest(t, conn, req)

		resp := readResponse(t, conn)
		qr := resp.GetQuery()
		if qr == nil {
			t.Fatalf("request %d: expected Query response", i)
		}
		if !bytes.Equal(qr.Data, data) {
			t.Fatalf("request %d: got %v, want %v", i, qr.Data, data)
		}
	}
}

// --- helpers ---

func tempSocket(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	return filepath.Join(dir, "test.sock")
}

func sendRequest(t *testing.T, conn net.Conn, req *pb.Request) {
	t.Helper()
	data, err := proto.Marshal(req)
	if err != nil {
		t.Fatalf("marshal request: %v", err)
	}
	if err := WriteFrame(conn, data); err != nil {
		t.Fatalf("write frame: %v", err)
	}
}

func readResponse(t *testing.T, conn net.Conn) *pb.Response {
	t.Helper()
	frame, err := ReadFrame(conn)
	if err != nil {
		t.Fatalf("read frame: %v", err)
	}
	var resp pb.Response
	if err := proto.Unmarshal(frame, &resp); err != nil {
		t.Fatalf("unmarshal response: %v", err)
	}
	return &resp
}

func TestMain(m *testing.M) {
	os.Exit(m.Run())
}
