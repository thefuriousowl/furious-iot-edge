#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "used through the private request assembler by a production TCP client in a later reviewed slice"
    )
)]
pub(crate) mod read_holding_registers;
