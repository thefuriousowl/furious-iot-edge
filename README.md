# furious-iot-edge

`furious-iot-edge` is a Rust-based industrial IoT edge runtime focused on
protocol correctness, predictable ownership, and low-allocation hot paths.

The first protocol target is Modbus TCP.

> Why furious? Because "just poll a few registers" is never just polling a few registers.

## Status

Early development.

Current Modbus TCP support includes:

- Persistent TCP sessions
- Serialized FC03 and FC04 transactions
- Caller-owned operation definitions
- Multiple operations on one connection
- MBAP framing and response correlation
- Validation of Transaction ID, Protocol ID, Unit ID, function code, MBAP Length, and Byte Count
- Fixed-size request and response buffers
- Register decoding without per-transaction heap allocation
- Register results and Modbus exception outcomes
- Preservation of unknown exception codes
- Typed protocol and I/O errors

Scripted TCP tests cover normal FC03/FC04 transactions, multiple operations on one
connection, valid exceptions followed by successful transactions, response mismatch
handling, malformed exception responses, invalid MBAP Length values, incomplete MBAP
headers, truncated response PDUs, and Byte Count validation.

Function codes other than FC03/FC04, scheduling, and reconnect behavior are not
implemented yet.

## Development

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings