mod mbap;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "used by a production Modbus TCP client in a later reviewed slice"
    )
)]
mod request;
