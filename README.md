# furious-iot-edge

Industrial IoT edge runtime in Rust. Early development.

# Status
- Simple single shot happy path for ModbusTCP read FC01,02,03,04 and write FC05,06,15,16
# Run
run example:
```bash
    cargo run --examples 00_simple_modbus_tcp_read_once             # oneshot FC01,02,03,04 happy path
    cargo run --examples 01_simple_modbus_tcp_single_write_once     # oneshot FC05,06 happy path
    cargo run --examples 02_simple_modbus_tcp_multiple_write_once   # oneshot FC01,02,03,04 happy path
```

LICENSE (MIT)