# furious-iot-edge

Industrial IoT edge runtime in Rust. Early development.

> Why furious? Because "just poll a few registers" is never just polling a few registers.

Currently supports Modbus TCP FC03/FC04 polling over persistent connections,
per-operation scan intervals, and reconnect after transaction I/O errors.

Protocol handling uses fixed-size buffers and keeps valid Modbus exceptions as
protocol outcomes rather than transport errors.

15 scripted TCP tests currently cover the Modbus transaction path.
Reconnect has also been manually verified by stopping and restarting the server.

```bash
cargo run
cargo test