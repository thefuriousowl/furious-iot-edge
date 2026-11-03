# furious-iot-edge

Industrial IoT edge runtime in Rust. Early development.

# Status
- Simple single shot happy path for ModbusTCP read FC01,02,03,04 and write FC05,06,15,16
# Run
run example:
```bash
    cargo run --example toy                                       # one-shot FC01,02,03,04,05,06,15,16
    cargo run --example toy -- 127.0.0.1:10502                     # optional server address
    cargo run --example 00_simple_modbus_tcp_read_once             # oneshot FC01,02,03,04 happy path
    cargo run --example 01_simple_modbus_tcp_single_write_once     # oneshot FC05,06 happy path
    cargo run --example 02_simple_modbus_tcp_multiple_write_once   # oneshot FC15,16 happy path
```

`toy` connects once, reads four requests, and writes four requests. The default
server address is `127.0.0.1:10502`. Its entry point is `examples/toy/main.rs`;
it uses the modules in `src/` until they are extracted into a library crate.

LICENSE (MIT)
