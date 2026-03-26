# hotmint-evm-rpc

Ethereum JSON-RPC server for the Hotmint EVM chain.

Implements `eth_*`, `net_*`, `web3_*` methods compatible with MetaMask, Foundry, and Hardhat. Runs as a standalone [axum](https://crates.io/crates/axum) HTTP server.

## Supported Methods

| Method | Description |
|:-------|:------------|
| `eth_chainId` | Chain ID |
| `eth_blockNumber` | Latest block height |
| `eth_getBalance` | Account balance |
| `eth_getTransactionCount` | Account nonce |
| `eth_getCode` | Contract bytecode |
| `eth_getStorageAt` | Storage slot value |
| `eth_gasPrice` | Base fee |
| `eth_estimateGas` | Gas estimate (21k for transfers) |
| `eth_maxPriorityFeePerGas` | Priority fee suggestion |
| `eth_sendRawTransaction` | Submit signed tx + gossip to peers |
| `eth_getBlockByNumber` | Block stub |
| `eth_feeHistory` | Fee history |
| `eth_syncing` | Sync status |
| `eth_accounts` | Empty (no managed keys) |
| `net_version` | Chain ID as string |
| `web3_clientVersion` | Client version |

## Transaction Gossip

When `eth_sendRawTransaction` succeeds, the transaction is broadcast to all connected peers via `NetworkSink::broadcast_tx()`. This ensures transactions submitted to any node reach the entire network.

## License

GPL-3.0-only
