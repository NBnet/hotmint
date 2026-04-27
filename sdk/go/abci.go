package hotmint

import (
	pb "github.com/rust-util-collections/hotmint/sdk/go/proto/abci"
)

// Application is the interface that Go applications must implement to be
// driven by the Hotmint consensus engine over IPC.
//
// This mirrors the Rust ApplicationHandler trait. The consensus engine sends
// requests via a Unix socket and the Go server dispatches them to this interface.
type Application interface {
	// Info returns the application's last committed height and app hash.
	Info() *pb.AppInfo

	// InitChain initializes the application from genesis state and returns the
	// initial app hash.
	InitChain(appState []byte) ([]byte, error)

	// CreatePayload builds the payload bytes for a new block proposal.
	CreatePayload(ctx *pb.BlockContext) []byte

	// ValidateBlock validates a proposed block before voting.
	ValidateBlock(block *pb.Block, ctx *pb.BlockContext) bool

	// ValidateTx validates a single transaction for mempool admission.
	// Returns (ok, priority, gas_wanted) — priority determines mempool ordering,
	// gas_wanted is used for per-block gas truncation.
	// ctx may be nil.
	ValidateTx(tx []byte, ctx *pb.TxContext) (bool, uint64, uint64)

	// ExecuteBlock executes an entire block and returns validator updates and events.
	ExecuteBlock(txs [][]byte, ctx *pb.BlockContext) (*pb.EndBlockResponse, error)

	// OnCommit is called when a block is committed to the chain.
	OnCommit(block *pb.Block, ctx *pb.BlockContext) error

	// OnEvidence is called when equivocation is detected.
	OnEvidence(proof *pb.EquivocationProof) error

	// OnOfflineValidators is called at epoch boundaries for validators whose
	// commit participation fell below the liveness threshold.
	OnOfflineValidators(offline []*pb.OfflineEvidence) error

	// ExtendVote returns an optional vote extension for the given block.
	// The boolean return indicates whether an extension is present.
	ExtendVote(block *pb.Block, ctx *pb.BlockContext) ([]byte, bool)

	// VerifyVoteExtension verifies another validator's vote extension.
	VerifyVoteExtension(extension []byte, blockHash []byte, validator uint64) bool

	// Query queries application state.
	// Returns a QueryResult containing the data, optional Merkle proof, and height.
	Query(path string, data []byte) (*QueryResult, error)

	// State sync callbacks.
	ListSnapshots() []*pb.SnapshotInfo
	LoadSnapshotChunk(height uint64, chunkIndex uint32) []byte
	OfferSnapshot(snapshot *pb.SnapshotInfo) uint32
	ApplySnapshotChunk(chunk []byte, chunkIndex uint32) uint32

	// TracksAppHash reports whether the app maintains deterministic app hashes.
	TracksAppHash() bool
}

// QueryResult holds the response from an Application.Query call.
type QueryResult struct {
	Data   []byte
	Proof  []byte // optional Merkle proof
	Height uint64
}

// BaseApplication provides default no-op implementations of all Application methods.
// Embed this in your application struct and override only the methods you need.
type BaseApplication struct{}

func genesisHash() []byte { return make([]byte, 32) }

func NewEndBlockResponse() *pb.EndBlockResponse {
	return &pb.EndBlockResponse{AppHash: genesisHash()}
}

const (
	SnapshotOfferAccept uint32 = 0
	SnapshotOfferReject uint32 = 1
	SnapshotOfferAbort  uint32 = 2

	ChunkApplyAccept uint32 = 0
	ChunkApplyRetry  uint32 = 1
	ChunkApplyAbort  uint32 = 2
)

func (BaseApplication) Info() *pb.AppInfo {
	return &pb.AppInfo{LastBlockAppHash: genesisHash()}
}
func (BaseApplication) InitChain(_ []byte) ([]byte, error)                 { return genesisHash(), nil }
func (BaseApplication) CreatePayload(_ *pb.BlockContext) []byte            { return nil }
func (BaseApplication) ValidateBlock(_ *pb.Block, _ *pb.BlockContext) bool { return true }
func (BaseApplication) ValidateTx(_ []byte, _ *pb.TxContext) (bool, uint64, uint64) {
	return true, 0, 0
}
func (BaseApplication) ExecuteBlock(_ [][]byte, _ *pb.BlockContext) (*pb.EndBlockResponse, error) {
	return NewEndBlockResponse(), nil
}
func (BaseApplication) OnCommit(_ *pb.Block, _ *pb.BlockContext) error            { return nil }
func (BaseApplication) OnEvidence(_ *pb.EquivocationProof) error                  { return nil }
func (BaseApplication) OnOfflineValidators(_ []*pb.OfflineEvidence) error         { return nil }
func (BaseApplication) ExtendVote(_ *pb.Block, _ *pb.BlockContext) ([]byte, bool) { return nil, false }
func (BaseApplication) VerifyVoteExtension(_ []byte, _ []byte, _ uint64) bool     { return true }
func (BaseApplication) Query(_ string, _ []byte) (*QueryResult, error) {
	return &QueryResult{}, nil
}
func (BaseApplication) ListSnapshots() []*pb.SnapshotInfo            { return nil }
func (BaseApplication) LoadSnapshotChunk(_ uint64, _ uint32) []byte  { return nil }
func (BaseApplication) OfferSnapshot(_ *pb.SnapshotInfo) uint32      { return SnapshotOfferReject }
func (BaseApplication) ApplySnapshotChunk(_ []byte, _ uint32) uint32 { return ChunkApplyAbort }
func (BaseApplication) TracksAppHash() bool                          { return true }
