# bench-consensus

Consensus throughput benchmark for the [Hotmint](https://github.com/NBnet/hotmint) BFT consensus framework.

Measures consensus throughput (blocks/sec) by building and running 4 separate `cluster-node` processes over real P2P for ~10s. The nodes run `NoopApplication`, whose `create_payload` returns an empty vector, so the measurement isolates consensus from application execution. (The 1 KB fixed payload belongs to `bench-ipc`.)

## Run

```bash
cargo run --release -p bench-consensus
```

## License

GPL-3.0-only
