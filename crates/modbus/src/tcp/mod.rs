#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "used by a production request assembler in a later reviewed slice"
    )
)]
mod mbap;
