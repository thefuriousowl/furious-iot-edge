# furious-iot-edge

Industrial IoT edge runtime in Rust. Early development.

# Status
- Simple single shot happy path for ModbusTCP read FC01,02,03,04 and write FC05,06,15,16
# Run
run example:
```bash
    cargo run --example 00_simple_modbus_tcp_read_once             # oneshot FC01,02,03,04 happy path
    cargo run --example 01_simple_modbus_tcp_single_write_once     # oneshot FC05,06 happy path
    cargo run --example 02_simple_modbus_tcp_multiple_write_once   # oneshot FC15,16 happy path
```

Owner scratch programs use `examples/toy_<name>.rs`, with one `main()` per file.
Cargo discovers each file as a separate example; `examples/toy_*.rs` is ignored
by Git, so these local scratch programs are not included in a fresh clone.

Run a local scratch program from the project root:

```bash
    cargo run --example toy_modbus_tcp
    cargo run --example toy_modbus_tcp -- 127.0.0.1:10502
    cargo run --example toy_mpsc_black_box
```

`toy_modbus_tcp` connects once, reads four requests, and writes four requests.
Its default server address is `127.0.0.1:10502`. It uses the modules in `src/`
until they are extracted into a library crate.

LICENSE (MIT)
