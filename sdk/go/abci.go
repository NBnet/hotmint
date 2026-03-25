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

	// Query queries application state.
	// Returns a QueryResult containing the data, optional Merkle proof, and height.
	Query(path string, data []byte) (*QueryResult, error)
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

func (BaseApplication) CreatePayload(_ *pb.BlockContext) []byte { return nil }
func (BaseApplication) ValidateBlock(_ *pb.Block, _ *pb.BlockContext) bool { return true }
func (BaseApplication) ValidateTx(_ []byte, _ *pb.TxContext) (bool, uint64, uint64) {
	return true, 0, 0
}
func (BaseApplication) ExecuteBlock(_ [][]byte, _ *pb.BlockContext) (*pb.EndBlockResponse, error) {
	return &pb.EndBlockResponse{}, nil
}
func (BaseApplication) OnCommit(_ *pb.Block, _ *pb.BlockContext) error { return nil }
func (BaseApplication) OnEvidence(_ *pb.EquivocationProof) error      { return nil }
func (BaseApplication) Query(_ string, _ []byte) (*QueryResult, error) {
	return &QueryResult{}, nil
}
