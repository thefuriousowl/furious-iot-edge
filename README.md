# furious-iot-edge

`furious-iot-edge` is a Rust-based industrial IoT edge runtime focused on
predictable ownership, low-allocation hot paths, and protocol correctness.

The first protocol target is Modbus TCP.

> Why furious? Because "just poll a few registers" is never just polling a few registers.

## Status

Early development.

The current Modbus TCP prototype supports:

- Persistent TCP sessions
- Serialized FC03 transactions
- Caller-owned FC03 operation definitions, borrowed by the session
- Modbus TCP MBAP framing
- Transaction ID correlation
- Protocol ID and Unit ID validation
- Fixed-size request and response buffers
- Register decoding without per-poll heap allocation
- Returned register data and Modbus exception outcomes
- Preservation of unrecognized exception codes

Additional function codes, scheduling, and reconnect behavior are not implemented yet.

## Development

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```
