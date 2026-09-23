# demo

Minimal consensus demo for the [Hotmint](https://github.com/NBnet/hotmint) BFT consensus framework.

Builds the `cluster-node` binary and spawns 4 separate OS processes in a temp directory (running `NoopApplication`), then polls each node's RPC `status` every 3s for 30s, printing the committed height and view.

## Run

```bash
cargo run -p hotmint-demo
```

## License

GPL-3.0-only
