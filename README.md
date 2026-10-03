# furious-iot-edge

`furious-iot-edge` is a Rust-based industrial IoT edge runtime focused on
predictable ownership, low-allocation hot paths, and protocol correctness.

The first protocol target is Modbus TCP.

> Why furious? Because "just poll a few registers" is never just polling a few registers.

## Status

Early development.

The current Modbus TCP prototype supports:

- Persistent TCP sessions
- Serialized FC03 (holding registers) and FC04 (input registers) transactions
- Caller-owned operation definitions with typed function codes, borrowed by the session
- Multiple operations with different Unit IDs, function codes, addresses, and quantities on one connection
- Modbus TCP MBAP framing
- Transaction ID correlation
- Protocol ID, Unit ID, and response function code validation
- Fixed-size request and response buffers
- Register decoding without per-poll heap allocation
- Returned register data and Modbus exception outcomes
- Preservation of unrecognized exception codes

Six scripted TCP tests cover request bytes, decoded register values, multiple
operations on one connection, valid FC03/FC04 exceptions followed by successful
transactions, and rejection of mismatched normal and exception function codes.

Function codes other than FC03/FC04, scheduling, and reconnect behavior are not
implemented yet.

## Development

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```
