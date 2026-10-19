# furious-iot-edge

Industrial IoT edge runtime in Rust. Early development.

> Why furious? Everything will be furious when developing in Rust. Including owner's head

- **Runtime:** Polls Modbus TCP (FC03/FC04) over one persistent connection.
  Exceptions are protocol outcomes, not errors. Last good snapshot survives exceptions and reconnects.
- **Control plane:** Loads config from SQLite (`vgateway -> device -> edge_operation`)
  and prepares a runtime command. Bad device/operation configs are skipped and reported,
  not fatal.

They are connected in the `command_modbus_tcp` example; `main` does not receive commands yet.

## Design

- `[u16; 125]` buffers: FC03/FC04 responses are protocol-bounded, so no per-poll allocation.
- One request in flight per connection: responses are matched before the next request.

## Run
Sample DB:
If `example.db` already exists, applying the seed fails on `UNIQUE`. Remove it first.
Then:

    sqlite3 local-data/example.db < local-data/schema.sql
    sqlite3 local-data/example.db < local-data/seed.sql

Then:

    cargo test                               # no Modbus server needed
    cargo run --example poll_modbus_tcp      # needs a simulator at 127.0.0.1:10502, Unit IDs 1 and 2
    cargo run --example command_modbus_tcp   # needs a simulator at 127.0.0.1:10502, Unit IDs 1 and 2

## Limitations
- FC03/FC04 only.
- `main` does not receive commands yet; run the runtime through the examples.
