# hotmint-evm-precompile

Custom EVM precompiled contracts for the Hotmint EVM chain.

Bridges the EVM execution layer to the native Hotmint staking system via precompile calls at reserved addresses.

## Precompiles

| Address | Name | Description |
|:--------|:-----|:------------|
| `0x0...0800` | Staking | Validator registration, delegation, undelegation, reward queries |

## Staking Precompile

Exposes `hotmint-staking` functionality to Solidity contracts via ABI-encoded function calls:

| Selector | Function | Description |
|:---------|:---------|:------------|
| `0x...` | `register(pubkey, power)` | Register as validator |
| `0x...` | `delegate(validator, amount)` | Delegate stake |
| `0x...` | `undelegate(validator, amount)` | Undelegate stake |

The staking state is shared between the precompile and the consensus engine via `SharedStakingState` (`Arc<Mutex<StakingState>>`).

## License

GPL-3.0-only
