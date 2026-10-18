# furious-iot-edge

Industrial IoT edge runtime in Rust. Early development.

> Why furious? everything will be furious when developing in Rust. Including owner's head

- **Runtime:** polls Modbus TCP (FC03/FC04) over one persistent connection.
  Exceptions are protocol outcomes, not errors. Last good snapshot survives exceptions and reconnects.
- **Control plane:** loads config from SQLite (`vgateway -> device -> edge_operation`)
  and prepares a runtime command. Bad device/operation configs are skipped and reported,
  not fatal.

The two parts are not connected yet.

## Design

- `[u16; 125]` buffers: FC03/FC04 responses are protocol-bounded, so no per-poll allocation.
- One request in flight per connection: responses are matched before the next request.

## Run

    cargo test                               # no Modbus server needed
    cargo run --example poll_modbus_tcp      # needs a simulator at 127.0.0.1:10502, Unit IDs 1 and 2

Sample DB: `sqlite3 local-data/example.db < local-data/schema.sql` then `< local-data/seed.sql`.

## Limitations

- Prepared commands are not executed yet.
- FC03/FC04 only.
- Control plane has no automated tests yet.
