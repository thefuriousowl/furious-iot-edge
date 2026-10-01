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
- Modbus TCP MBAP framing
- Transaction ID correlation
- Protocol ID and Unit ID validation
- Fixed-size request and response buffers
- Register decoding without per-poll heap allocation

Operation modeling, response outcomes, additional function codes, scheduling,
and reconnect behavior are still under development.

## Development

```bash
cargo fmt --check
cargo test