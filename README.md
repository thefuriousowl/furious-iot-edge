# furious-iot-edge

Industrial IoT edge runtime in Rust. Early development.

> Why furious? Because "just poll a few registers" is never just polling a few registers.

Currently supports Modbus TCP FC03/FC04 polling over persistent connections,
with per-operation Unit IDs, function codes, addresses, quantities, and scan intervals.

Protocol handling uses fixed-size buffers and keeps valid Modbus exceptions as
protocol outcomes rather than transport errors.

Each scheduled operation retains its latest successful register snapshot and
acquisition time. Exceptions and errors do not overwrite it, and it survives reconnect.

Failure handling:

- socket reads time out instead of waiting forever on a silent device
- invalid responses that leave the stream aligned are skipped; polling continues
  on the same connection
- I/O errors and invalid responses that may leave the stream misaligned trigger reconnect

18 scripted TCP tests cover the Modbus transaction path, read timeout,
snapshot retention after exceptions, and continued polling after invalid responses.
Reconnect has been manually verified by stopping and restarting the server.

Known limitations: initial connection failure terminates the executable, and
function codes other than FC03/FC04 are not implemented.

```bash
cargo run
cargo test
```
