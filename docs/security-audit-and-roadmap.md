# Hotmint 完整审查报告：安全漏洞、工程缺陷与演进路线图

> **报告版本：** 基于 Hotmint v0.7 / CometBFT v0.38
> **生成日期：** 2026-03-24 | **最后审计：** 2026-03-25
> **来源：** CometBFT 特性差异分析 + 两轮代码安全审计
> **用途：** 作为长期演进路线图的参考基准，每次迭代后可对照更新完成状态（将 `[ ]` 改为 `[x]`，部分完成标记 `[~]`）。

---

## 1. 执行摘要

| 维度 | CometBFT v0.38 | Hotmint v0.7 |
|------|---------------|-------------|
| 语言 | Go | Rust |
| 共识算法 | Tendermint（三阶段 BFT） | HotStuff-2（双链提交 BFT） |
| 成熟度 | 生产级，Cosmos 生态主力引擎 | 工程原型，架构完整但生产配套不完善 |
| 核心优势 | 生态完备、工具链丰富、协议成熟 | 延迟更低、架构更模块化、内存安全 |
| 主要短板 | 三阶段投票延迟高、Go GC 长尾抖动 | 安全防护薄弱、状态同步/轻客户端/事件订阅缺失 |

Hotmint 凭借 **Rust + HotStuff-2 + litep2p** 的组合，在核心共识算法和架构现代化上具备超越 CometBFT 的潜力。当前差距主要集中在两个层面：
- **安全防护层：** 存在若干可被主动利用的漏洞（Eclipse 攻击、Spam DoS、Panic 向量）
- **工程完备性层：** 对标 CometBFT 缺少状态同步、轻客户端、事件订阅等生产级基础设施

---

## 2. 共识协议核心对比

### 2.1 算法层

| 对比项 | CometBFT v0.38 | Hotmint v0.7 |
|--------|---------------|-------------|
| 协议族 | Tendermint BFT | HotStuff-2（arXiv:2301.03253） |
| 投票阶段 | 三阶段：Propose → Pre-vote → Pre-commit | 两链：Propose → Vote → QC → Vote2 → DC |
| 提交规则 | Pre-commit 超 2/3 后单块提交 | Double Certificate（两轮 2f+1）后双链提交 |
| 视图切换（ViewChange） | 复杂：需收集 prevotes，存在 Nil 投票路径 | 线性：Wish 消息聚合为 TimeoutCert，无额外开销 |
| 提议者选举 | 加权轮询（weighted round-robin） | 简单轮询（`view % validator_count`）⚠️ 缺权重 |
| 网络复杂度 | O(n²) 广播 | O(n²)（同阶，但阶段更少） |
| 理论延迟 | ~2 个网络往返（三阶段） | ~2 个网络往返（两阶段，各含 QC 聚合） |
| BFT 容错边界 | f < n/3 | f < n/3 |
| 时间戳来源 | BFT Time（验证者投票时间中位数） | 由提议者指定（无 BFT Time 共识）⚠️ |

### 2.2 安全机制

| 对比项 | CometBFT v0.38 | Hotmint v0.7 |
|--------|---------------|-------------|
| 重放攻击保护 | Chain ID 编码在签名域 | Blake3(chain_id) 注入所有签名 ✅ |
| 状态分叉检测 | App hash 链 + ABCI 校验 | App hash 链（每块头携带上一块执行结果）✅ |
| 双签检测 | 完整证据收集 + 网络广播 | 局部检测（同 view+type 不同 block_hash），不持久化 ⚠️ |
| WAL 崩溃恢复 | 有 Write-Ahead Log，精确重放 | 无 WAL，依赖 vsdb `PersistentConsensusState` ⚠️ |
| 锁定机制 | polkaValue / 轮次锁 | `locked_qc`（安全性等价）✅ |
| 跨 Epoch 投票重放防护 | epoch 编码在签名或状态机切换保护中 | `signing_bytes` 仅含 `chain_id_hash + view + block_hash`，缺少 epoch_number ⚠️ |

---

## 3. 应用接口（ABCI 层）对比

### 3.1 接口方法全量对比

| 方法/回调 | CometBFT ABCI++ v0.38 | Hotmint `Application` Trait | 状态 |
|-----------|----------------------|----------------------------|------|
| 区块提议构造 | `PrepareProposal` | `create_payload` | ✅ 语义等价 |
| 区块提议验证 | `ProcessProposal` | `validate_block` | ✅ 语义等价 |
| 交易执行 | `FinalizeBlock` | `execute_block` | ✅ 语义等价 |
| 交易预校验 | `CheckTx` | `validate_tx` | ✅ 语义等价 |
| 块提交回调 | `Commit`（含 app_hash） | `on_commit` | ✅ |
| 证据惩罚 | `FinalizeBlock.misbehavior[]` | `on_evidence(EquivocationProof)` | ✅ |
| 状态查询 | `Query` | `query(path, data)` | ✅ |
| **投票扩展附加** | **`ExtendVote`** | **缺失** | ❌ |
| **投票扩展验证** | **`VerifyVoteExtension`** | **缺失** | ❌ |
| 快照创建 | `ListSnapshots` / `LoadSnapshotChunk` | **缺失** | ❌ |
| 快照接收 | `OfferSnapshot` / `ApplySnapshotChunk` | **缺失** | ❌ |
| 应用信息 | `Info`（含 last_block_height） | `tracks_app_hash` 间接实现 | ⚠️ |
| 初始化创世 | `InitChain` | 无显式接口（应用构造时处理） | ⚠️ |

### 3.2 跨进程通信

| 对比项 | CometBFT v0.38 | Hotmint v0.7 |
|--------|---------------|-------------|
| 同进程接口 | Go interface | Rust trait ✅ |
| 跨语言/跨进程 | gRPC（`.proto` 多语言 SDK） | Unix domain socket + CBOR（`hotmint-abci`）⚠️ |
| IPC 超时保护 | gRPC 内置超时 | **无超时**，`spawn_blocking` 可能永久挂起 ❌ |

---

## 4. P2P 网络层对比

| 对比项 | CometBFT v0.38 | Hotmint v0.7 |
|--------|---------------|-------------|
| 底层框架 | 自研 MConnTransport（多路复用 TCP） | litep2p（Rust，衍生自 Polkadot 生态）✅ |
| 消息路由 | Reactor 模型 | Notification + Request-Response 协议分离 ✅ |
| 点对点加密 | SecretConnection（Noise） | litep2p 内置 Noise/TLS ✅ |
| Peer 发现 | PEX Reactor + 种子节点 | PEX 协议（`/hotmint/pex/1`）✅ |
| 验证者连接保护 | Persistent Peers 优先保留槽位 | **无验证者槽保留**，可被女巫节点占满 ❌ |
| 连接管理 | 持久/非持久对等体 + 拨号调度 | 维护循环（10s）+ 退避，无优先级 ⚠️ |
| 消息压缩 | 内部协议处理 | zstd 压缩，压缩侧 `.unwrap()` 无保护 ⚠️ |

---

## 5. 内存池（Mempool）对比

| 对比项 | CometBFT v0.38 | Hotmint v0.7 |
|--------|---------------|-------------|
| 数据结构 | 并发链表 + LRU 去重缓存 | `VecDeque`（FIFO）+ `HashSet`（Blake3）|
| 排序策略 | **优先级队列**（应用返回 priority 字段） | **FIFO 严格按序** ❌ |
| 容量控制 | `max_txs`（数量）+ `max_txs_bytes`（总字节） | `max_size`（数量）+ `max_tx_bytes`（单笔）|
| 溢出驱逐 | 低优先级交易被驱逐 | 直接拒绝新交易 ❌ |
| 重验证 | 出块后对悬挂交易重跑 `CheckTx` | 无重验证 ❌ |
| Gas 感知 | 应用返回 `gas_wanted`，Mempool 据此驱逐 | 无 Gas 感知 ❌ |
| API 速率限制 | 支持限速配置 | **无任何速率限制**，可被瞬间打满 ❌ |
| P2P 广播 | Flood Mempool，对等体 Gossip | 仅 RPC 接受，无 P2P gossip ❌ |

---

## 6. 区块同步（Block Sync）对比

| 对比项 | CometBFT v0.38 | Hotmint v0.7 |
|--------|---------------|-------------|
| 实现方式 | Block Sync Reactor，多节点并发下载 | 单节点串行批次拉取（max 100 blocks/batch）⚠️ |
| 验证强度 | 每块验证 commit 签名（2/3 以上） | 依赖 `app_hash` 对比（可选）+ QC 验证 |
| 追赶后切换 | 自动切换为共识 reactor | `sync_to_tip` 完成后启动共识引擎 ✅ |

---

## 7. 状态同步（State Sync）对比

| 对比项 | CometBFT v0.38 | Hotmint v0.7 |
|--------|---------------|-------------|
| 能力 | **完整支持**：快照列举、分块下载、校验、应用 | **完全缺失** ❌ |
| 应用侧接口 | `ListSnapshots`、`LoadSnapshotChunk`、`OfferSnapshot`、`ApplySnapshotChunk` | 未实现 |
| 典型加入时间 | 分钟级（下载快照） | 与链龄正相关（数小时至数天）|

---

## 8. 轻客户端（Light Client）对比

| 对比项 | CometBFT v0.38 | Hotmint v0.7 |
|--------|---------------|-------------|
| 实现 | 完整：二分搜索验证、不信任区段跳跃 | **完全缺失** ❌ |
| Merkle 证明输出 | `Query` 返回 Merkle proof | RPC `query` 无 Merkle proof ❌ |
| 跨链基础 | IBC 协议依赖轻客户端 | 无法实现标准 IBC ❌ |

---

## 9. RPC / API 层对比

| 对比项 | CometBFT v0.38 | Hotmint v0.7 |
|--------|---------------|-------------|
| 传输协议 | HTTP + WebSocket（标准） | 原始 TCP 换行 JSON（非标准）❌ |
| 事件订阅 | WebSocket `subscribe`（丰富过滤语法） | **缺失** ❌ |
| 方法数量 | 20+ 方法 | 5 个方法 ⚠️ |
| 交易查询 | 按 hash 查询、事件索引 | 不支持 ❌ |

---

## 10. 观测性与运维对比

| 对比项 | CometBFT v0.38 | Hotmint v0.7 |
|--------|---------------|-------------|
| Prometheus Metrics | 丰富（共识轮次、P2P 流量、Mempool 深度等）| 基础（view、height、blocks、votes、timeouts）✅ |
| 结构化日志 | slog/zap | `tracing` crate ✅ |
| WAL 崩溃恢复 | 有 WAL，精确恢复到崩溃前投票状态 | 无 WAL ⚠️ |

---

## 11. 惩罚与证据机制对比

| 对比项 | CometBFT v0.38 | Hotmint v0.7 |
|--------|---------------|-------------|
| 双签证据 | `DuplicateVoteEvidence`（持久化 + gossip）| `EquivocationProof`（检测即触发，不持久化）⚠️ |
| 证据广播 | P2P 层 gossip，全网可见 | 无证据广播 ❌ |
| 证据持久化 | 证据池持久化，重启不丢 | 内存检测，重启丢失 ❌ |
| 离线惩罚 | 支持（`downtime` 逻辑）| 无 ❌ |

---

## 12. 特性全景汇总

| 特性 | CometBFT v0.38 | Hotmint v0.7 | 差距等级 |
|------|:--------------:|:------------:|:--------:|
| BFT 共识核心 | ✅ | ✅ | 无 |
| 加权提议者选举 | ✅ | ❌ | 中 |
| BFT Time | ✅ | ❌ | 低 |
| ABCI 准入接口（Prepare/Process）| ✅ | ✅ | 无 |
| **Vote Extensions** | ✅ | ❌ | **高** |
| **快照状态同步** | ✅ | ❌ | **高** |
| **轻客户端验证** | ✅ | ❌ | **高** |
| **Merkle 证明输出** | ✅ | ❌ | **高** |
| **WebSocket 事件订阅** | ✅ | ❌ | **高** |
| **优先级内存池** | ✅ | ❌ | **高** |
| Mempool P2P Gossip | ✅ | ❌ | 中 |
| Mempool 重验证 | ✅ | ❌ | 中 |
| Block Sync（多节点并发）| ✅ | ⚠️ 单节点 | 中 |
| WAL 崩溃恢复 | ✅ | ⚠️ 部分 | 中 |
| 证据持久化与广播 | ✅ | ❌ | 中 |
| 标准 HTTP JSON-RPC | ✅ | ❌ | 中 |
| 交易/区块历史查询 | ✅ | ❌ | 中 |
| IBC 跨链能力 | ✅（需轻客户端）| ❌ | **高** |
| 完整运维 CLI | ✅ | ⚠️ 基础 | 低 |

---

## 13. 完整待修复事项清单

以下条目按**真实风险优先级**排列，融合了 CometBFT 功能差距分析与代码安全审计两个来源的发现。每条附有类型标签：`[安全漏洞]` / `[工程缺陷]` / `[功能缺失]`。

---

### 🔴 Critical — 安全漏洞（阻塞正式上线）

#### [x] C-1. Eclipse 攻击：P2P 验证者连接槽缺乏保护 `[安全漏洞]`

**位置：** `crates/hotmint-network/src/service.rs`

**问题：** 网络层按 `max_peers` 限制总连接数。当连接数达到上限后，新入站连接一律被拒绝。攻击者可用大量女巫节点（Sybil Nodes）占满连接槽，导致合法验证者之间无法建立 P2P 链路，共识网络失去活性。

**修复方案：**
- 在入站握手阶段检查对方是否在当前 `ValidatorSet` 中
- 若是验证者节点，即使达到 `max_peers` 上限，也强制挤掉一个低信誉的非验证者连接为其腾槽
- 为验证者节点维护独立的「保留槽（Reserved Slots）」，数量不低于 `validator_count`

**风险等级：** 🔴 高危 — 可导致网络级活性失败（Liveness Failure）

> **实现状态：✅ 基本完成。** 已在入站握手阶段检查 `peer_to_validator`，验证者即使超 `max_peers` 也不被拒。尚未实现：独立保留槽计数器、主动驱逐非验证者连接为验证者腾位。

---

#### [~] C-2. FIFO Mempool DoS：垃圾交易阻断合法交易 `[安全漏洞 × 功能缺失]`

**位置：** `crates/hotmint-mempool/src/lib.rs`、`crates/hotmint-api/src/rpc.rs`

**问题 A（Spam DoS）：** Mempool 是无优先级的 FIFO 队列（默认上限 10,000 条），API 层无任何速率限制。攻击者可在瞬间向 RPC 接口提交 10,000 笔体积微小但满足 `validate_tx` 的无用交易，把队列塞满。后续合法交易全部被拒绝，实质上实现了针对链交易通道 DoS 攻击。

**问题 B（DeFi 不可用）：** 无 Gas/Priority 排序意味着无法支持手续费竞价机制，DeFi 应用无法正常运行。

**修复方案：**
- **Mempool 重构：** 将 `VecDeque` 替换为 `BinaryHeap`（按 `priority` 排序），在 `validate_tx` 返回值中增加 `priority: u64` 和 `gas_wanted: u64`；池满时驱逐优先级最低的交易
- **来源限额：** 限制单一 IP/PeerId 的同时占用比例（如最多占总容量 10%）
- **API 限速：** 在 `hotmint-api` 的 RPC 层对 `submit_tx` 增加每 IP 速率限制（如令牌桶算法）
- **`collect_payload` 扩展：** 增加 `max_gas_per_block` 按 gas 累计截断

**关键文件：** `crates/hotmint-mempool/src/lib.rs`、`crates/hotmint-consensus/src/application.rs`

**风险等级：** 🔴 高危 — 可实现链上交易通道 DoS

> **实现状态：⚠️ 部分完成。** 已完成：BTreeSet 优先级队列 + RBF、`TxValidationResult { valid, priority }` 返回值、池满驱逐最低优先级、令牌桶限速（100 tx/sec per connection）。尚未实现：`gas_wanted` 字段、per-IP/PeerId 来源限额（当前限速仅 per-connection，攻击者可多连接绕过）、`collect_payload` 的 `max_gas_per_block` 截断。

---

#### [~] C-3. 证据广播缺失：双签者可免于惩罚 `[安全漏洞 × 功能缺失]`

**位置：** `crates/hotmint-consensus/src/vote_collector.rs`、`crates/hotmint-consensus/src/engine.rs`（约第 991 行）

**问题：** `vote_collector.rs` 能正确检测双签并生成 `EquivocationProof`，引擎随后调用 `app.on_evidence(proof)` 传给应用层。但 Hotmint 没有将证据广播至全网的机制——如果检测到作恶的节点不是当前 Leader，该证据仅停留在本地应用进程中。作恶者针对部分非出块节点进行双签欺骗，这些证据无法上链，可完全躲避 Slashing 惩罚。此外证据不持久化，节点重启后证据丢失。

**修复方案：**
- 在 `hotmint-network` 增加 `/hotmint/evidence/1` P2P 广播协议
- 引擎检测到 `EquivocationProof` 后，**立即**通过 P2P 广播给全网
- Leader 打包下一个 Block 时，**强制**将收集到的未上链 Evidence 嵌入 Block Header 或 Payload
- 在 `hotmint-storage` 增加 `EvidenceStore`（vsdb 持久化），重启后不丢失

**关键文件：** `crates/hotmint-consensus/src/engine.rs`、`crates/hotmint-storage/`、`crates/hotmint-network/src/service.rs`

**风险等级：** 🔴 高危 — 惩罚机制形同虚设，恶意验证者可无成本双签

> **实现状态：⚠️ 部分完成。** 已完成：`ConsensusMessage::Evidence` 消息类型、`broadcast_evidence()` 通过现有 notification 协议广播（非独立协议）、`EvidenceStore` trait（put/get_pending/mark_committed/all）、`MemoryEvidenceStore` 内存实现、引擎检测到双签后立即广播+存储、收到 gossip 证据后存储并通知应用层。尚未实现：vsdb 持久化存储（当前仅内存，重启丢失）、Leader 打包证据进 Block（代码注释 "full block inclusion is a later step"）、`mark_committed` 从未被调用。

#### [x] C-4. Proposal ancestor constraint missing `[Safety Violation]` ✅

**Location:** `crates/hotmint-consensus/src/view_protocol.rs` (`on_proposal`, ~line 200)

**Problem:** `on_proposal` never verifies that `block.parent_hash == justify.block_hash`, nor that the parent block exists in the store. A Byzantine leader can propose a block forking from an arbitrary point in the chain — honest nodes will accept and store it, potentially voting for a block that does not extend the certified chain.

**Fix:**
- Before accepting a proposal, verify `block.parent_hash == justify.block_hash`
- Verify parent block exists in store (or is genesis) before voting

**Severity:** 🔴 Critical — violates chain extension safety property

---

#### [x] C-5. Vote2Msg path missing vote_type check — phase confusion `[Safety Violation]` ✅

**Location:** `crates/hotmint-consensus/src/engine.rs` (~line 975, `ConsensusMessage::Vote2Msg`)

**Problem:** The `Vote2Msg` handler does not verify `vote.vote_type == VoteType::Vote2`. The `VoteMsg` handler correctly checks `vote.vote_type == VoteType::Vote` and rejects mismatches, but `Vote2Msg` accepts any vote_type. A malicious peer can send a `Vote2Msg` containing a Vote1-phase vote, bypassing Vote1 path constraints and potentially forming a DoubleCert from votes in the wrong phase.

**Fix:** Add `if vote.vote_type != VoteType::Vote2 { return Ok(()); }` at the top of the Vote2Msg handler, mirroring the VoteMsg guard.

**Severity:** 🔴 Critical — phase confusion can produce invalid DoubleCert

---

#### [x] C-6. Evidence gossip accepted without cryptographic verification `[Safety Violation]` ✅

**Location:** `crates/hotmint-consensus/src/engine.rs` (~line 1098, `ConsensusMessage::Evidence`)

**Problem:** When the engine receives an `Evidence(proof)` message via gossip, it calls `app.on_evidence()` and stores the proof without verifying the two conflicting signatures. A malicious peer can forge an `EquivocationProof` with arbitrary signatures, triggering application-layer slashing logic against an innocent validator.

**Fix:**
- Before accepting evidence, verify both `signature_a` and `signature_b` using the alleged validator's public key and the corresponding signing_bytes
- Reject and drop the proof if either signature is invalid

**Severity:** 🔴 Critical — forged evidence can slash innocent validators

---

#### [x] C-7. `apply_commit` + `persist_state` not atomic — crash recovery gap `[Engineering Defect]` ✅

**Location:** `crates/hotmint-consensus/src/engine.rs` (`apply_commit` ~line 1296, `persist_state` ~line 1475)

**Problem:** `apply_commit` executes blocks (mutating application state) and flushes to the block store, but consensus state (`last_committed_height`, `current_view`, `locked_qc`) is only persisted later in `persist_state()` during `advance_view_to`. If the node crashes between `apply_commit` completing and `persist_state` being called:
- Application state reflects committed blocks
- Block store reflects committed blocks
- But on-disk consensus state still shows the previous height/view
- On restart the node may re-execute already-committed blocks, causing state divergence

**Fix:** Call `persist_state()` at the end of `apply_commit` (after `s.flush()`), or adopt a write-ahead log (WAL) that records the commit intent before executing.

**Severity:** 🔴 Critical — crash window causes irrecoverable state divergence

---

### 🟡 High — 工程安全（生产部署前应修复）

#### [x] H-1. O(N) 签名验证 CPU DoS 风险 `[安全漏洞]` ✅

**位置：** `crates/hotmint-crypto/src/aggregate.rs`

**问题：** 当前「聚合签名」实质上是把 N 个 Ed25519 签名拼接（Concatenation）后循环调用 `ed25519_dalek::Verifier::verify`，时间复杂度 O(N)。每次收到 QC、DC 及区块同步时都需要全量遍历验证。攻击者可频繁发送带有随机数据的看似合法 QC，迫使节点消耗巨量 CPU，导致视图切换超时（Liveness 失败）。

**修复方案（二选一）：**
1. **方案 A（长期）：** 引入真正的聚合签名机制（如 BLS12-381），将 N 个签名验证压缩为一次 pairing 操作，验证成本 O(1)
2. **方案 B（短期）：** 将 `verify_aggregate` 移交至 `tokio::task::spawn_blocking` 专用 CPU 线程池，避免阻塞共识引擎主事件循环；同时对未知来源的 QC/DC 消息增加来源鉴权（只接受来自已知验证者 PeerId 的消息）

**关键文件：** `crates/hotmint-crypto/src/aggregate.rs`

**风险等级：** 🟡 中危 — 在大型验证者集合（100+ 节点）下可触发 Liveness 失败

> **实现状态：✅ 100% 完成。** 方案 B 全部落地：`verify_aggregate` 改用 `ed25519_dalek::verify_batch`（Bos-Coster 批验证）；`verify_message` / `validate_double_cert` / Wish QC 验证均包裹在 `tokio::task::block_in_place` 中；签名域绑定 epoch 防重放。

---

#### [x] H-2. `pending_epoch` 强制解包 Panic 向量 `[工程缺陷]` ✅

**位置：** `crates/hotmint-consensus/src/engine.rs`（Epoch 切换逻辑）

**问题：** Epoch 切换代码中存在 `self.pending_epoch.take().unwrap()` 强制解包。如果共识状态在崩溃恢复或异常边缘情况后未能正确注入 `pending_epoch`（例如：应用层返回了 ValidatorUpdates 但进程随之崩溃重启），此处 `unwrap()` 将在共识引擎触及特定视图高度时引发不可恢复的 Panic，导致该节点在该高度永久宕机。

**修复方案：**
- 将 `unwrap()` 替换为 `ok_or`/`expect` 配合 `Result` 传播
- 若 `pending_epoch` 缺失，fallback 到安全状态（保持当前 Epoch 继续运行）或向应用层重新请求状态同步
- 增加针对此路径的单元测试（模拟崩溃重启后的 Epoch 切换）

**关键文件：** `crates/hotmint-consensus/src/engine.rs`

**风险等级：** 🟡 中危 — 特定崩溃恢复路径下可引发不可恢复的节点宕机

---

#### [x] H-3. zstd 压缩端 `unwrap()` Panic 向量 `[工程缺陷]` ✅

**位置：** `crates/hotmint-network/src/codec.rs`

**问题：** 代码对解压（Decompress）端正确设置了 `MAX_DECOMPRESSED_SIZE` 限制，但压缩（Compress）端使用 `zstd::encode_all(..., ZSTD_LEVEL).unwrap()`。当操作系统内存耗尽或某些极端超大载荷触发 zstd 内部错误时，这个 `unwrap()` 将直接令底层网络服务崩溃，导致节点下线。

**修复方案：**
- 将 `zstd::encode_all` 返回的 `Result` 向上传播
- 压缩失败时，丢弃该消息或断开对应客户端连接，**禁止** Panic 传播至主进程

**关键文件：** `crates/hotmint-network/src/codec.rs`

**风险等级：** 🟡 中危 — 特殊网络负载下可触发节点下线

---

#### [x] H-4. ABCI IPC 通信无超时保护 `[工程缺陷]` ✅

**位置：** `crates/hotmint-abci/src/client.rs`

**问题：** 同步帧读写中，若应用端进程（Go/其他语言实现）僵死但 Unix socket 未断开，目前没有显式的 `ReadTimeout` / `WriteTimeout` 设置。这会导致 Rust 共识引擎在 `tokio::task::spawn_blocking` 中永久挂起，阻塞整个共识进程出块，实质上使链停止。

**修复方案：**
- 为底层 `UnixStream` 设置严格的读写超时（建议与 `base_timeout_ms` 挂钩，或设置固定的 5s 超时）
- 超时后将 IPC 失败作为致命错误上报，触发应用层重连或节点重启流程

**关键文件：** `crates/hotmint-abci/src/client.rs`

**风险等级：** 🟡 中危 — 应用端进程异常时可使共识引擎永久停机

---

#### [x] H-5. 投票签名缺少 `epoch_number`，存在跨 Epoch 重放风险 `[安全隐患]` ✅

**位置：** `crates/hotmint-types/src/vote.rs`（`signing_bytes` 方法）

**问题：** 当前 `signing_bytes` 包含 `chain_id_hash + view + block_hash`。`view` 是全局单调递增的，短期内是安全的。但若未来实现跨 Epoch 的状态重置、验证者集合大幅变动或链分叉修复，旧 Epoch 中合法生成的签名可能被用于构造新 Epoch 下的虚假投票（跨 Epoch 重放攻击）。

**修复方案：**
- 在 `signing_bytes` 中显式加入 `epoch_number` 字段
- 验证投票时同步校验 `epoch_number` 与当前 Epoch 一致
- 此变更涉及线上数据格式，需版本化迁移

**关键文件：** `crates/hotmint-types/src/vote.rs`

**风险等级：** 🟡 低-中危 — 当前模式下无法触发，但影响未来扩展的安全性

#### [x] H-6. P2P handshake empty — no chain/genesis/version isolation `[Engineering Defect]` ✅

**Location:** `crates/hotmint-network/src/service.rs` (~line 205, 410)

**Problem:** The litep2p notification protocol is initialized with `.with_handshake(vec![])` (empty) and `.with_auto_accept_inbound(true)`. Inbound substreams are unconditionally accepted via `ValidationResult::Accept`. Any peer can connect and inject consensus messages regardless of chain_id, genesis hash, or protocol version. This allows cross-chain message injection in multi-chain deployments.

**Fix:** Include `chain_id_hash + protocol_version` in the handshake bytes; in `ValidateSubstream`, verify the handshake matches before accepting.

**Severity:** 🟡 High — cross-chain message injection in multi-network environments

---

#### [x] H-7. Sync replay epoch transition applies immediately, runtime delays to start_view `[Engineering Defect]` ✅

**Location:** `crates/hotmint-consensus/src/sync.rs` (~line 413) vs `crates/hotmint-consensus/src/engine.rs` (~line 1431)

**Problem:** During `replay_blocks`, epoch transitions take effect immediately (`*state.current_epoch = Epoch::new(...)`) after the committing block. During normal consensus, the engine stores a `pending_epoch` and only applies it when `new_view >= e.start_view`. This creates a semantic mismatch: sync-replaying nodes use the new validator set immediately, while live consensus nodes use it only after `start_view`. Blocks in the gap window may be verified against different validator sets.

**Fix:** `replay_blocks` should defer the epoch transition to `start_view`, or replay blocks in the gap window using the old validator set and switch at the correct view.

**Severity:** 🟡 High — validator set mismatch during/after sync can cause verification failures

---

#### [x] H-8. `SharedStoreAdapter` panics on lock contention (`try_read/try_write`) `[Engineering Defect]` ✅

**Location:** `crates/hotmint-consensus/src/store.rs` (lines 48–97)

**Problem:** Every `BlockStore` method in `SharedStoreAdapter` uses `self.0.try_read().expect("store read lock contended")` or `try_write().expect(...)`. If the `tokio::sync::RwLock` is held by another task at the time of the call, `try_*` returns `Err` and `.expect()` panics, crashing the node. This is used by the sync responder path which runs concurrently with the consensus engine.

**Fix:** Replace `try_read().expect()` with `.read().await` (or `blocking_read()` in sync contexts), or accept `Result` and propagate errors gracefully.

**Severity:** 🟡 High — concurrent access causes node crash

---

#### [x] H-9. Node binary defaults `evidence_store: None` — evidence system inert `[Engineering Defect]` ✅

**Location:** `crates/hotmint/src/bin/node.rs` (~line 724)

**Problem:** The production node binary constructs `EngineConfig` with `evidence_store: None`. Despite all the evidence infrastructure (EvidenceStore trait, MemoryEvidenceStore, broadcast_evidence, gossip handling), the store is never wired in. `handle_equivocation` silently skips storage; gossip evidence silently skips storage.

**Fix:** Initialize `evidence_store: Some(Box::new(MemoryEvidenceStore::new()))` in the node binary. For persistence, implement a vsdb-backed store.

**Severity:** 🟡 High — evidence system is dead code in production

---

#### [x] H-10. HTTP RPC rate limiter created per-request — effectively disabled `[Engineering Defect]` ✅

**Location:** `crates/hotmint-api/src/http_rpc.rs` (~line 95)

**Problem:** `json_rpc_handler` creates a fresh `TxRateLimiter::new(TX_RATE_LIMIT_PER_SEC)` on every HTTP request. Each request gets a full token allowance, making the rate limit meaningless. An attacker can submit unlimited `submit_tx` calls by sending unlimited HTTP requests.

**Fix:** Store a shared `TxRateLimiter` (or per-IP map) in `HttpRpcState` and pass it to each request handler. The TCP RPC server correctly creates one limiter per connection.

**Severity:** 🟡 High — mempool spam via HTTP endpoint

---

#### [x] H-11. ABCI IPC `ValidateTx` returns only `bool`, client hardcodes `priority: 0` `[Engineering Defect]` ✅

**Location:** `crates/hotmint-abci-proto/proto/abci.proto` (ValidateTxResponse), `crates/hotmint-abci/src/client.rs` (~line 172)

**Problem:** The IPC wire protocol (`ValidateTxResponse { bool ok }`) does not carry a `priority` field. The Rust ABCI client maps `ok=true` to `TxValidationResult { valid: true, priority: 0 }`. This means out-of-process applications (Go, etc.) cannot signal transaction priority, rendering the priority mempool queue, eviction, and RBF features inoperative for ABCI apps.

**Fix:** Extend `ValidateTxResponse` with `uint64 priority` (and optionally `uint64 gas_wanted`). Update client to read and forward to `TxValidationResult`.

**Severity:** 🟡 Medium — priority mempool disabled for all ABCI applications

---

#### [x] H-12. Sync replay does not persist `commit_qc` `[Engineering Defect]` ✅

**Location:** `crates/hotmint-consensus/src/sync.rs` (`replay_blocks`, ~line 375)

**Problem:** `replay_blocks` stores blocks via `state.store.put_block()` but never calls `put_commit_qc()` for synced blocks, despite the QC being available in the input tuple `(Block, Option<QuorumCertificate>)`. After sync, the node cannot serve commit QCs to other syncing peers or to the light client RPC (`get_commit_qc`), creating a "sync hole" that degrades network resilience.

**Fix:** After `put_block`, call `state.store.put_commit_qc(block.height, qc.clone())` when `qc.is_some()`.

**Severity:** 🟡 Medium — synced nodes cannot serve commit proofs to peers or light clients

---

### 🟢 P0 — 功能演进：生产链必需

#### [~] P0-1. 标准 HTTP/WebSocket RPC + 事件订阅 `[功能缺失]`

**当前差距：** 原始 TCP 换行 JSON 协议对前端 dApp 不友好；无 WebSocket 事件订阅使 DApp 无法实时监听链上状态。

**建议实现：**
- 将 `hotmint-api` 底层替换为 `axum` 或 `jsonrpsee`，提供标准 HTTP + WebSocket
- 引入事件总线（`tokio::sync::broadcast`），在 `on_commit` 时发布 `BlockEvent` / `TxEvent`
- 实现 `subscribe` RPC，支持按 `tx.hash`、`block.height`、自定义 tag 过滤
- 增加 `get_tx`（按 hash 查状态）、`get_block_results` 等常用方法

**关键文件：** `crates/hotmint-api/src/rpc.rs`、`crates/hotmint-api/src/types.rs`

> **实现状态：⚠️ 部分完成。** 已完成：axum HTTP `POST /` + WS `GET /ws`、`broadcast::Sender<ChainEvent>` 事件总线、`NewBlock` 事件实时推送、`get_header` / `get_commit_qc` RPC。尚未实现：`get_tx`（按 hash 查交易）、`get_block_results`、`subscribe` RPC（当前仅 WS 推所有事件，无过滤）、`TxCommitted` / `EpochChange` 事件类型。

---

### 🟢 P1 — 功能演进：网络健壮性

#### [x] P1-1. 快照状态同步（State Sync via Snapshots） `[功能缺失]` ✅

**当前差距：** 新节点必须从高度 0 全量重放，链运行数月后入网时间不可接受，是招募新验证者的障碍。

**建议实现：**
```rust
// Application trait 新增
fn list_snapshots(&self) -> Vec<Snapshot>;
fn load_snapshot_chunk(&self, height: u64, chunk_index: u32) -> Vec<u8>;
fn offer_snapshot(&self, snapshot: &Snapshot, app_hash: &[u8]) -> OfferSnapshotResult;
fn apply_snapshot_chunk(&self, chunk: Vec<u8>, index: u32) -> ApplyChunkResult;
```
- 利用 `vsdb` 内置 MPT 根哈希作为快照可信锚点
- P2P 同步协议增加 `GetSnapshotMeta` / `GetSnapshotChunk` 消息类型
- 节点启动配置 `state_sync = true` 时优先走快照通道

**关键文件：** `crates/hotmint-consensus/src/application.rs`、`crates/hotmint-consensus/src/sync.rs`

> **实现状态：✅ 100% 完成。** Application trait 4 个快照方法全部就位；P2P 消息 `SyncRequest::GetSnapshots` / `GetSnapshotChunk` 及对应 Response 已定义；`sync_via_snapshot()` 完整实现（请求快照列表→选最新→offer→逐块下载→apply→更新高度）。`state_sync` 配置标志可由应用层控制。

---

#### [x] P1-2. 加权提议者选举（Weighted Proposer Selection） `[功能缺失]` ✅

**当前差距：** `view % validator_count` 不考虑质押权重，对不均匀质押分布不公平。

**建议实现：**
- 在 `ValidatorSet` 中启用 `voting_power` 字段
- 实现类 CometBFT 的加权轮询算法（按 `voting_power` 比例递增每个验证者的优先级分，取最高分者为 proposer）
- 向后兼容现有 Epoch 结构

**关键文件：** `crates/hotmint-consensus/src/leader.rs`、`crates/hotmint-types/src/validator.rs`

---

### 🟢 P2 — 功能演进：生态扩展

#### [~] P2-1. 轻客户端验证协议（Light Client Protocol） `[功能缺失]`

**当前差距：** 无法支持 IBC 跨链通讯，无法支持移动端钱包无信任验证。

**建议实现：**
- 基于现有 `QuorumCertificate`（已含 2f+1 聚合签名）设计轻客户端验证路径
- `get_block` RPC 可选返回 `commit_qc` + Merkle proof
- 增加 `verify_header` RPC（仅验证 QC 签名和验证者集合变更）
- 提供独立 `hotmint-light` crate 供第三方集成

**关键文件：** `crates/hotmint-api/`、`crates/hotmint-types/src/certificate.rs`

> **实现状态：⚠️ 部分完成。** 已完成：`hotmint-light` crate（`LightClient` + `verify_header` + `update_validator_set`，含 4 项单元测试）、RPC `get_header` / `get_commit_qc` 方法。尚未实现：Merkle proof 输出（`query` 返回值无 proof 字段）、轻客户端验证未通过 RPC 直接暴露。

---

#### [x] P2-2. ABCI++ Vote Extensions（投票扩展） `[功能缺失]`

**当前差距：** 无法实现内置预言机、阈值签名聚合、抗 MEV 机制。

**建议实现：**
- 在 `Vote` 结构增加 `extension: Option<Vec<u8>>`
- 在 `Vote2` 阶段前新增两个应用回调：
  ```rust
  fn extend_vote(&self, block: &Block, ctx: &BlockContext) -> Option<Vec<u8>>;
  fn verify_vote_extension(&self, ext: &[u8], validator: ValidatorId) -> bool;
  ```
- 在 `Double Certificate` 中聚合所有验证者的 extension
- 下一轮 `create_payload` 可读取上一轮的 extension 集合

**关键文件：** `crates/hotmint-types/src/message.rs`、`crates/hotmint-consensus/src/view_protocol.rs`

> **实现状态：✅ 基本完成。** 已完成：`Vote.extension: Option<Vec<u8>>` 字段、`extend_vote()` / `verify_vote_extension()` 应用回调（含默认 no-op）、引擎在 Vote2 创建前调用 `extend_vote`、收到 Vote2 时调用 `verify_vote_extension` 验证。尚未实现：DoubleCert 中显式聚合所有 extension、下一轮 `create_payload` 直接读取上轮 extension 集合（需应用层自行追踪）。

---

## 14. 全量优先级汇总表

| ID | 严重度 | 描述 | 状态 | 缺失项 |
|----|--------|------|:----:|--------|
| C-1 | 🔴 高危 | Eclipse 攻击：验证者连接槽无保护 | ✅ | 独立保留槽计数、主动驱逐非验证者 |
| C-2 | 🔴 高危 | FIFO Mempool DoS + 无 API 速率限制 | ⚠️ | per-IP 来源限额 |
| C-3 | 🔴 高危 | 证据广播缺失，双签者可逃脱惩罚 | ⚠️ | vsdb 持久化、证据打包进区块 payload |
| H-1 | 🟡 中危 | O(N) 签名验证 CPU DoS 风险 | ✅ | — |
| H-2 | 🟡 中危 | `pending_epoch.unwrap()` Panic 向量 | ✅ | — |
| H-3 | 🟡 中危 | zstd 压缩端 `unwrap()` Panic 向量 | ✅ | — |
| H-4 | 🟡 中危 | ABCI IPC 无读写超时，可致永久挂起 | ✅ | — |
| H-5 | 🟡 低-中 | 签名缺 `epoch_number`，跨 Epoch 重放风险 | ✅ | — |
| **C-4** | 🔴 Critical | Proposal missing parent_hash == justify.block_hash check | ✅ | — |
| **C-5** | 🔴 Critical | Vote2Msg no vote_type == Vote2 guard — phase confusion | ✅ | — |
| **C-6** | 🔴 Critical | Evidence gossip accepted without signature verification | ✅ | — |
| **C-7** | 🔴 Critical | apply_commit + persist_state not atomic — crash gap | ✅ | — |
| **H-6** | 🟡 High | Empty P2P handshake — no chain/version isolation | ✅ | — |
| **H-7** | 🟡 High | Sync epoch transition immediate vs runtime delayed | ✅ | — |
| **H-8** | 🟡 High | SharedStoreAdapter try_read/try_write panics on contention | ✅ | — |
| **H-9** | 🟡 High | Node binary defaults evidence_store: None | ✅ | — |
| **H-10** | 🟡 High | HTTP rate limiter per-request — effectively disabled | ✅ | — |
| **H-11** | 🟡 Medium | ABCI IPC ValidateTx returns bool, priority hardcoded 0 | ✅ | — |
| **H-12** | 🟡 Medium | Sync replay doesn't persist commit_qc | ✅ | — |
| P0-1 | 🟢 P0 | 标准 HTTP/WS RPC + 事件订阅系统 | ⚠️ | `get_tx`、`get_block_results`、`subscribe` 过滤、更多事件类型 |
| P1-1 | 🟢 P1 | 快照状态同步（State Sync） | ✅ | — |
| P1-2 | 🟢 P1 | 加权提议者选举 | ✅ | — |
| P2-1 | 🟢 P2 | 轻客户端验证协议 | ⚠️ | Merkle proof 输出、RPC 暴露验证接口 |
| P2-2 | 🟢 P2 | ABCI++ Vote Extensions | ✅ | DC 聚合 extension、next-round payload 读取（可由应用层自行追踪） |
| R-1 | 🟢 低危 | RwLock 公平锁 RPC 拥塞 | 📋 | 无锁只读快照 / watch channel / parking_lot 迁移 |

---

## 15. Medium-term Improvements (Second Audit Round)

Items from the second code audit. Low severity but relevant to long-term throughput.

#### [ ] R-1. `tokio::sync::RwLock` Fair Lock Causes RPC Congestion Under High Concurrency `[Performance]`

**Location:** `crates/hotmint-api/src/rpc.rs`, `crates/hotmint-consensus/src/engine.rs`

**Problem:** The consensus engine and the RPC layer share `Arc<tokio::sync::RwLock<Box<dyn BlockStore>>>`. `tokio::sync::RwLock` is a fair lock — when a writer (`store.write().await`) is queued, new readers are also blocked. If the RPC endpoint is public-facing and hit by bursty `get_block` / `get_commit_qc` traffic, accumulated read locks can force the consensus engine's `put_block` writes to queue, slightly slowing block confirmation.

**Current mitigation:** All lock holds are synchronous HashMap lookups with no `.await` points (microsecond-level). The `try_propose` write lock is already scoped to release before any `.await`. Actual contention probability is very low.

**Suggested optimization paths (medium-term):**
- **Option A:** Provide RPC with a lock-free read-only snapshot handle (leveraging VSDB snapshot capabilities), fully decoupling RPC reads from consensus writes
- **Option B:** Publish latest block header/height via `Arc<tokio::sync::watch::Sender>`, making basic status queries zero-contention
- **Option C:** Migrate to `parking_lot::RwLock` (guards are `Send`, usable across `.await`), or `dashmap` / lock-free concurrent structures

**Severity:** Low — only impacts TPS under extreme RPC concurrency (thousands of QPS)

---

## 16. Long-term Vision: Substrate Pallets Dimensionality-Reduction Porting

> **Prerequisite:** All work in this section is blocked until the infrastructure in sections 13–15 is fully stable (all ⚠️ items resolved to ✅, R-1 addressed with at least one option). Current stage is planning only — no implementation until prerequisites are met.

### 16.1 Strategic Rationale

Hotmint has a modern consensus core (HotStuff-2), high-performance async runtime (Tokio), and a clean stateless `Application` trait. However, building application-layer logic (tokens, PoS, governance) from scratch carries enormous engineering cost and audit risk.

Parity's (Polkadot) **Substrate FRAME Pallets** represent the industry's most complete and battle-tested pure-Rust blockchain business logic library, audited by top security firms over multiple years.

**Core approach:** Use LLM semantic extraction and code rewriting to strip Substrate's most stable Pallets of their macro system (`#[pallet::*]`) and Wasm/`no_std` constraints, porting them into Hotmint's `std` + `vsdb` + `serde` environment. This delivers production-grade business modules at minimal engineering cost.

### 16.2 Dimensionality-Reduction Mapping Rules

| Substrate (FRAME) Primitive | Hotmint Target | Notes |
|:---|:---|:---|
| `#[pallet::storage] StorageMap<K, V>` | `vsdb::MapxOrd<K, V>` | Strip macros, use vsdb persistent key-value storage directly |
| `DispatchError` / `#[pallet::error]` | `ruc::Result<()>` | Unified `ruc` chained error handling |
| `#[pallet::event]` | `hotmint_types::ReceiptLog` | Events become block execution receipt logs |
| `sp_runtime::traits::Currency` | Plain `std` Rust trait | Keep core abstractions, remove `no_std`/SCALE bindings |
| SCALE Codec (`Encode`/`Decode`) | `serde` (CBOR/JSON) | Web-friendly standard serialization |
| `no_std` environment | `std` environment | Hotmint runs natively as an OS process, no Wasm boundary |
| `ensure_root` / `ensure_signed` | Transaction signer public key verification | Permission modifiers map to cryptographic identity checks |

### 16.3 Three-Phase Porting Roadmap

#### Phase 1: Foundation Economy

**Goal:** A chain supporting account system, fungible token issuance, and transfers.

| Component | Source | Core Capabilities | Integration Point |
|-----------|--------|-------------------|-------------------|
| `pallet-balances` | Substrate | Balance management, transfer, reserve, lock | Called within `execute_block`, state written to vsdb |
| `pallet-assets` | Substrate | Multi-asset (ERC-20-like) mint, burn, freeze | Same as above |
| `pallet-timestamp` | Substrate | Block timestamp consensus | Integrates with `BlockContext.view` / proposer time |

**Prerequisites:** P0-1 (HTTP RPC) complete for dApp frontend interaction; C-2 (`gas_wanted`) complete for fee model support.

#### Phase 2: Governance & Native PoS Integration

**Goal:** Replace the current static `ValidatorSet` with a real DPoS/PoS economic model.

| Component | Source | Core Capabilities | Integration Point |
|-----------|--------|-------------------|-------------------|
| `pallet-staking` | Substrate | Nomination, validator election (Phragmen), slashing calculation | Drives epoch transitions via `EndBlockResponse.validator_updates` |
| `pallet-session` | Substrate | Key rotation and validator set updates at epoch boundaries | Integrates with `pending_epoch` mechanism |
| `pallet-multisig` | Substrate | Multisig wallets, delayed execution | `validate_tx` + `execute_block` |

**Prerequisites:** C-3 evidence on-chain complete (slashing requires on-chain verifiable equivocation proofs); `hotmint-staking` crate serves as porting base.

#### Phase 3: Advanced Contract Platform

**Goal:** Full-featured AppChain / Rollup Sequencer.

| Component | Source | Core Capabilities | Integration Point |
|-----------|--------|-------------------|-------------------|
| `pallet-evm` (SputnikVM) | Substrate | 100% Ethereum smart contract compatibility | Strip Substrate shell, embed SputnikVM within `execute_block` |

**Prerequisites:** Phase 1 account/balance system as native token backend for EVM; existing `examples/evm-chain` (using `revm`) serves as reference implementation.

### 16.4 Implementation Standards

1. **AI Prompt Template Library:** Develop standardized prompt templates — input: Substrate source code; output: Hotmint-conformant `vsdb` + `Application` trait code
2. **State Root Integrity:** All state mutations must write through `vsdb` to ensure correct `app_hash` computation
3. **Security Audit Transfer:** Although business logic originates from audited Substrate code, ported code requires secondary security review, focusing on:
   - Integer overflow checks (`checked_add`/`checked_sub`) preserved completely
   - Permission modifiers correctly mapped to transaction signer public key verification
   - Storage key namespaces properly isolated (no cross-pallet state pollution)

### 16.5 Competitive Positioning

Post-completion Hotmint ecosystem position:

| Dimension | vs CometBFT/Tendermint | vs Cosmos SDK |
|-----------|----------------------|---------------|
| Consensus | HotStuff-2: lower latency, no GC tail-latency jitter | — |
| Business Logic | — | AI-ported Substrate Pallets: pure Rust, type-safe, no Keeper/Handler nesting |
| Smart Contracts | — | Native EVM compatibility (revm/SputnikVM) |
| Positioning | High-performance AppChain consensus engine | Next-gen AppChain + Rollup Sequencer full-stack solution |

---

## References

- [CometBFT v0.38 Documentation](https://docs.cometbft.com/v0.38/introduction/)
- [CometBFT ABCI++ Specification](https://docs.cometbft.com/v0.38/spec/abci/)
- [HotStuff-2 Paper](https://arxiv.org/abs/2301.03253)
- [Substrate FRAME Pallets Source](https://github.com/niccolocorsini/polkadot-sdk/tree/master/substrate/frame)
- [Hotmint Architecture](architecture.md)
- [Hotmint Application Trait Guide](application.md)
- [Hotmint Mempool & API](mempool-api.md)
