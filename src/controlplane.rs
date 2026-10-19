use std::{net::IpAddr, time::Duration};

use rusqlite::Connection;
use serde::Deserialize;

use crate::{
    command::{ModbusTcpCommand, ModbusTcpEdgeOperationCommand},
    db_model::{device::DeviceRow, edge_operation::EdgeOperationRow, vgateway::VGatewayRow},
    modbus_tcp::FunctionCode,
    scheduler::MIN_ALLOW_INTERVAL,
};

#[derive(Debug)]
pub enum PrepareError {
    Db(rusqlite::Error),
    NotFound {
        table: &'static str,
        key: String,
    },
    InvalidConfig {
        table: &'static str,
        id: String,
        reason: String,
    },
    Disabled {
        table: &'static str,
        id: String,
    },
    UnsupportedOperation {
        table: &'static str,
        function: u8,
        name: String,
        id: String,
    },
}
impl From<rusqlite::Error> for PrepareError {
    fn from(e: rusqlite::Error) -> Self {
        PrepareError::Db(e)
    }
}
#[derive(Debug, Deserialize)]
pub struct ModbusTcpDeviceConfig {
    pub unit_id: u8,
}
#[derive(Debug, Deserialize)]
pub struct ModbusTcpEdgeOperationConfig {
    function: u8,
    address: u16,
    quantity: u16,
    poll_interval_ms: u64,
}

#[derive(Debug, Deserialize)]
pub struct ModbusTcpVGatewayConfig {
    host: IpAddr,
    port: u16,
    request_timeout_ms: u64,
}
pub fn prepare_modbus_tcp(
    conn: &Connection,
    gateway_id: &str,
) -> Result<(ModbusTcpCommand, Vec<PrepareError>), PrepareError> {
    let vgateway_record_result = VGatewayRow::find_by_id(conn, gateway_id)?;

    let vgateway_record = vgateway_record_result.ok_or_else(|| PrepareError::NotFound {
        table: "vgateway",
        key: gateway_id.to_string(),
    })?;

    if !vgateway_record.enabled {
        return Err(PrepareError::Disabled {
            table: "vgateway",
            id: gateway_id.to_string(),
        });
    }

    if vgateway_record.driver != "modbus_tcp" {
        return Err(PrepareError::InvalidConfig {
            table: "vgateway",
            id: vgateway_record.id,
            reason: "this vgateway driver is not modbus_tcp".to_string(),
        });
    }

    let parse_vgateway_config =
        serde_json::from_value::<ModbusTcpVGatewayConfig>(vgateway_record.config).map_err(|e| {
            PrepareError::InvalidConfig {
                table: "vgateway",
                id: vgateway_record.id.clone(),
                reason: e.to_string(),
            }
        })?;
    let socket_address =
        std::net::SocketAddr::new(parse_vgateway_config.host, parse_vgateway_config.port);

    if parse_vgateway_config.request_timeout_ms == 0 {
        return Err(PrepareError::InvalidConfig {
            table: "vgateway",
            id: vgateway_record.id,
            reason: format!(
                "invalid request_timeout_ms. Expect > 0 found: {}",
                parse_vgateway_config.request_timeout_ms
            ),
        });
    }

    let child_devices_record = DeviceRow::find_all_by_gateway_id(conn, gateway_id, true)?;
    let mut err_collection = Vec::new();
    let mut edge_operation_commands = Vec::new();
    for dev in child_devices_record {
        let dev_config = match serde_json::from_value::<ModbusTcpDeviceConfig>(dev.config.clone()) {
            Ok(cfg) => cfg,
            Err(e) => {
                err_collection.push(PrepareError::InvalidConfig {
                    table: "device",
                    id: dev.id,
                    reason: e.to_string(),
                });
                continue;
            }
        };
        let child_operations_record = EdgeOperationRow::find_all_by_device_id(conn, &dev.id, true)?;

        for op in child_operations_record {
            let op_config =
                match serde_json::from_value::<ModbusTcpEdgeOperationConfig>(op.config.clone()) {
                    Ok(cfg) => cfg,
                    Err(e) => {
                        err_collection.push(PrepareError::InvalidConfig {
                            table: "edge_operation",
                            id: op.id,
                            reason: e.to_string(),
                        });
                        continue;
                    }
                };

            let (parse_fc, max_quantity) = match op_config.function {
                3 => (FunctionCode::ReadHoldingRegisters, 125),
                4 => (FunctionCode::ReadInputRegisters, 125),
                _ => {
                    err_collection.push(PrepareError::UnsupportedOperation {
                        table: "edge_operation",
                        name: op.name,
                        function: op_config.function,
                        id: op.id,
                    });
                    continue;
                }
            };

            if let Err(e) = validate_modbus_tcp_operation_config(
                &op.id,
                op_config.quantity,
                max_quantity,
                op_config.address,
                op_config.poll_interval_ms,
            ) {
                err_collection.push(e);
                continue;
            }

            edge_operation_commands.push(ModbusTcpEdgeOperationCommand {
                operation_id: op.id,
                unit_id: dev_config.unit_id,
                function_code: parse_fc,
                starting_address: op_config.address,
                quantity: op_config.quantity,
                poll_interval: Duration::from_millis(op_config.poll_interval_ms),
            });
        }
    }

    Ok((
        ModbusTcpCommand {
            gateway_id: vgateway_record.id,
            address: socket_address,
            request_timeout: Duration::from_millis(parse_vgateway_config.request_timeout_ms),
            operations: edge_operation_commands,
        },
        err_collection,
    ))
}

fn validate_modbus_tcp_operation_config(
    id: &str,
    quantity: u16,
    max_qty: u16,
    address: u16,
    poll_interval_ms: u64,
) -> Result<(), PrepareError> {
    //validate quantity
    if !(1..=max_qty).contains(&quantity) {
        return Err(PrepareError::InvalidConfig {
            table: "edge_operation",
            id: id.to_string(),
            reason: format!(
                "Invalid quantity. Expected 1-{} but found {}",
                max_qty, quantity
            ),
        });
    }
    // validate starting address + quantity in acceptable range
    if u32::from(quantity) + u32::from(address) > 65536 {
        return Err(PrepareError::InvalidConfig {
            table: "edge_operation",
            id: id.to_string(),
            reason: format!(
                "Invalid address range. Expected <= 65536 but found {}",
                u32::from(quantity) + u32::from(address)
            ),
        });
    }

    // validate poll_interval_ms must be at least 100 ms
    if Duration::from_millis(poll_interval_ms) < MIN_ALLOW_INTERVAL {
        return Err(PrepareError::InvalidConfig {
            table: "edge_operation",
            id: id.to_string(),
            reason: format!(
                "Invalid poll_interval_ms. Expected >= {:?} but found {} ms",
                MIN_ALLOW_INTERVAL, poll_interval_ms
            ),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rusqlite::Connection;
    use serde_json::json;

    use crate::{
        command::ModbusTcpEdgeOperationCommand,
        controlplane::{PrepareError, prepare_modbus_tcp},
        modbus_tcp::FunctionCode::ReadHoldingRegisters,
    };

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().expect("should create in-memory sqlite db");

        conn.execute_batch(include_str!("../local-data/schema.sql"))
            .expect("should create empty tables");
        conn
    }

    fn insert_gateway(conn: &Connection, id: &str, driver: &str, enabled: bool, config: &str) {
        conn.execute(
            "INSERT INTO vgateway (id, name, driver, config, enabled, created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, '2026-10-18T00:00:00Z', '2026-10-18T00:00:00Z')",
            [
                id,
                &format!("gw-{}", id),
                driver,
                config,
                &(enabled as i32).to_string(),
            ],
        )
        .expect("should write mock gateway");
    }

    fn insert_device(conn: &Connection, id: &str, gateway_id: &str, enabled: bool, config: &str) {
        conn.execute(
            "INSERT INTO device (id, gateway_id, name, slug, config, enabled, created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, '2026-10-18T00:00:00Z', '2026-10-18T00:00:00Z')",
            [
                id,
                gateway_id,
                &format!("{}-name", id),
                &format!("{}-slug", id),
                config,
                &(enabled as i32).to_string(),
            ]
        ).expect("should write mock device");
    }

    fn insert_operation(conn: &Connection, id: &str, device_id: &str, enabled: bool, config: &str) {
        conn.execute(
            "INSERT INTO edge_operation (id, device_id, name, config, enabled, created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, '2026-10-18T00:00:00Z', '2026-10-18T00:00:00Z')",
            [id, device_id, &format!("{}-name", id), config, &(enabled as i32).to_string()]
        ).expect("should write mock operation");
    }

    #[test]
    fn prepares_command_from_valid_modbus_tcp_config() {
        let conn = setup_db();
        let gw_id = "gw-1";
        let gw_config =
            &json!({ "host": "127.0.0.1", "port": 10502, "request_timeout_ms": 1500 }).to_string();
        insert_gateway(&conn, gw_id, "modbus_tcp", true, gw_config);

        let dev_id = "dev-1";
        let device_config = &json!({"unit_id":1}).to_string();
        insert_device(&conn, dev_id, gw_id, true, device_config);

        let op_id = "op-1";
        let op_config =
            &json!({"function":3,"address":0,"quantity":10,"poll_interval_ms":1000}).to_string();
        insert_operation(&conn, op_id, dev_id, true, op_config);

        let (cmd_result, skipped) =
            prepare_modbus_tcp(&conn, gw_id).expect("valid modbus tcp command");
        assert_eq!(cmd_result.address, "127.0.0.1:10502".parse().unwrap());
        assert_eq!(cmd_result.request_timeout, Duration::from_millis(1500));
        assert_eq!(cmd_result.gateway_id, gw_id);
        let expected_operation = ModbusTcpEdgeOperationCommand {
            operation_id: op_id.to_string(),
            unit_id: 1,
            function_code: ReadHoldingRegisters,
            starting_address: 0,
            quantity: 10,
            poll_interval: Duration::from_millis(1000),
        };
        assert_eq!(cmd_result.operations, vec![expected_operation]);
        assert!(skipped.is_empty());
    }

    #[test]
    fn operations_keep_unit_id_of_their_own_device() {
        let conn = setup_db();
        let gw_id = "gw-1";
        let gw_config =
            &json!({ "host": "127.0.0.1", "port": 10502, "request_timeout_ms": 1500 }).to_string();
        insert_gateway(&conn, gw_id, "modbus_tcp", true, gw_config);

        let dev_id_1 = "dev-1";
        let device_config_1 = &json!({"unit_id":1}).to_string();
        insert_device(&conn, dev_id_1, gw_id, true, device_config_1);

        let dev_id_2 = "dev-2";
        let device_config_2 = &json!({"unit_id":2}).to_string();
        insert_device(&conn, dev_id_2, gw_id, true, device_config_2);

        let op_id_1 = "op-1";
        let op_config =
            &json!({"function":3,"address":0,"quantity":10,"poll_interval_ms":1000}).to_string();
        insert_operation(&conn, op_id_1, dev_id_1, true, op_config);

        let op_id_2 = "op-2";
        let op_config =
            &json!({"function":3,"address":0,"quantity":10,"poll_interval_ms":1000}).to_string();
        insert_operation(&conn, op_id_2, dev_id_2, true, op_config);

        let (cmd_result, skipped) =
            prepare_modbus_tcp(&conn, gw_id).expect("valid modbus tcp command");
        assert_eq!(
            cmd_result
                .operations
                .iter()
                .find(|op| op.operation_id == op_id_1)
                .map(|op| op.unit_id)
                .expect("op-1 should be in command"),
            1
        );

        assert_eq!(
            cmd_result
                .operations
                .iter()
                .find(|op| op.operation_id == op_id_2)
                .map(|op| op.unit_id)
                .expect("op-2 should be in command"),
            2
        );

        assert!(skipped.is_empty())
    }
    #[test]
    fn excludes_disabled_devices_and_operations() {
        let conn = setup_db();
        let gw_id = "gw-1";
        let gw_config =
            &json!({ "host": "127.0.0.1", "port": 10502, "request_timeout_ms": 1500 }).to_string();
        insert_gateway(&conn, gw_id, "modbus_tcp", true, gw_config);

        let dev_id_1 = "dev-1";
        let device_config_2 = &json!({"unit_id":1}).to_string();
        insert_device(&conn, dev_id_1, gw_id, true, device_config_2);

        let dev_id_2 = "dev-2";
        let device_config_2 = &json!({"unit_id":2}).to_string();
        insert_device(&conn, dev_id_2, gw_id, false, device_config_2);

        let op_id_1 = "op-1";
        let op_config =
            &json!({"function":3,"address":0,"quantity":10,"poll_interval_ms":1000}).to_string();
        insert_operation(&conn, op_id_1, dev_id_1, true, op_config);

        let op_id_2 = "op-2";
        let op_config =
            &json!({"function":3,"address":0,"quantity":10,"poll_interval_ms":1000}).to_string();
        insert_operation(&conn, op_id_2, dev_id_1, false, op_config);

        let op_id_3 = "op-3";
        let op_config =
            &json!({"function":3,"address":0,"quantity":10,"poll_interval_ms":1000}).to_string();
        insert_operation(&conn, op_id_3, dev_id_2, true, op_config);

        let (cmd_result, skipped) =
            prepare_modbus_tcp(&conn, gw_id).expect("valid modbus tcp command");

        assert_eq!(cmd_result.operations.len(), 1);
        assert_eq!(cmd_result.operations[0].operation_id, op_id_1);
        assert!(skipped.is_empty());
    }

    #[test]
    fn skips_device_with_invalid_config_and_keeps_other_devices() {
        let conn = setup_db();
        let gw_id = "gw-1";
        let gw_config =
            &json!({ "host": "127.0.0.1", "port": 10502, "request_timeout_ms": 1500 }).to_string();
        insert_gateway(&conn, gw_id, "modbus_tcp", true, gw_config);

        let dev_id_1 = "dev-1";
        let device_config_1 = &json!({"unit_id":1}).to_string();
        insert_device(&conn, dev_id_1, gw_id, true, device_config_1);

        let op_id_1 = "op-1";
        let op_config_1 =
            &json!({"function":3,"address":0,"quantity":10,"poll_interval_ms":1000}).to_string();
        insert_operation(&conn, op_id_1, dev_id_1, true, op_config_1);

        let dev_id_2 = "dev-2";
        let device_config_2 = &json!({"unit":2}).to_string();
        insert_device(&conn, dev_id_2, gw_id, true, device_config_2);

        let op_id_2 = "op-2";
        let op_config_2 =
            &json!({"function":3,"address":0,"quantity":10,"poll_interval_ms":1000}).to_string();
        insert_operation(&conn, op_id_2, dev_id_2, true, op_config_2);

        let (cmd_result, skipped) =
            prepare_modbus_tcp(&conn, gw_id).expect("valid modbus tcp command");

        assert_eq!(cmd_result.operations.len(), 1);
        assert_eq!(cmd_result.operations[0].operation_id, op_id_1);
        assert!(skipped.len() == 1);
        match &skipped[0] {
            PrepareError::InvalidConfig {
                table,
                id,
                reason: _,
            } => {
                assert_eq!(*table, "device");
                assert_eq!(id, "dev-2");
            }
            other => panic!("Expected InvalidConfig, got {:?}", other),
        }
    }
    #[test]
    fn skips_unsupported_function_code_and_keeps_other_operations() {
        let conn = setup_db();
        let gw_id = "gw-1";
        let gw_config =
            &json!({ "host": "127.0.0.1", "port": 10502, "request_timeout_ms": 1500 }).to_string();
        insert_gateway(&conn, gw_id, "modbus_tcp", true, gw_config);

        let dev_id = "dev-1";
        let device_config = &json!({"unit_id":1}).to_string();
        insert_device(&conn, dev_id, gw_id, true, device_config);

        let op_id_1 = "op-1";
        let op_config_1 =
            &json!({"function":3,"address":0,"quantity":10,"poll_interval_ms":1000}).to_string();
        insert_operation(&conn, op_id_1, dev_id, true, op_config_1);

        let op_id_2 = "op-2";
        let op_config_2 =
            &json!({"function":99,"address":0,"quantity":10,"poll_interval_ms":1000}).to_string();
        insert_operation(&conn, op_id_2, dev_id, true, op_config_2);

        let (cmd_result, skipped) =
            prepare_modbus_tcp(&conn, gw_id).expect("valid modbus tcp command");

        assert_eq!(cmd_result.operations.len(), 1);
        assert_eq!(cmd_result.operations[0].operation_id, op_id_1);
        assert!(skipped.len() == 1);
        match &skipped[0] {
            PrepareError::UnsupportedOperation {
                table,
                function,
                name,
                id,
            } => {
                assert_eq!(*table, "edge_operation");
                assert_eq!(*function, 99);
                assert_eq!(name, "op-2-name");
                assert_eq!(id, "op-2");
            }
            other => panic!("Expected UnsupportedOperation, got {:?}", other),
        }
    }

    #[test]
    fn skips_operations_outside_modbus_limits() {
        let conn = setup_db();
        let gw_id = "gw-1";
        let gw_config =
            &json!({ "host": "127.0.0.1", "port": 10502, "request_timeout_ms": 1500 }).to_string();
        insert_gateway(&conn, gw_id, "modbus_tcp", true, gw_config);

        let dev_id = "dev-1";
        let device_config = &json!({"unit_id":1}).to_string();
        insert_device(&conn, dev_id, gw_id, true, device_config);

        let op_q0 = "op-q0";
        let op_config_q0 =
            &json!({"function":3,"address":0,"quantity":0,"poll_interval_ms":1000}).to_string();
        insert_operation(&conn, op_q0, dev_id, true, op_config_q0);

        let op_q126 = "op-q126";
        let op_config_q126 =
            &json!({"function":3,"address":0,"quantity":126,"poll_interval_ms":1000}).to_string();
        insert_operation(&conn, op_q126, dev_id, true, op_config_q126);

        let op_addr = "op-addr";
        let op_config_addr =
            &json!({"function":3,"address":65535,"quantity":2,"poll_interval_ms":1000}).to_string();
        insert_operation(&conn, op_addr, dev_id, true, op_config_addr);

        let op_poll = "op-poll";
        let op_config_poll =
            &json!({"function":3,"address":0,"quantity":2,"poll_interval_ms":99}).to_string();
        insert_operation(&conn, op_poll, dev_id, true, op_config_poll);

        let (cmd_result, skipped) =
            prepare_modbus_tcp(&conn, gw_id).expect("valid modbus tcp command");

        assert!(cmd_result.operations.is_empty());
        assert_eq!(skipped.len(), 4);
        for err in skipped {
            match err {
                PrepareError::InvalidConfig {
                    table,
                    id: _,
                    reason: _,
                } => {
                    assert_eq!(table, "edge_operation");
                }
                other => panic!("Expected InvalidConfig, got {:?}", other),
            }
        }
    }
    #[test]
    fn accepts_operations_at_modbus_limits() {
        let conn = setup_db();
        let gw_id = "gw-1";
        let gw_config =
            &json!({ "host": "127.0.0.1", "port": 10502, "request_timeout_ms": 1500 }).to_string();
        insert_gateway(&conn, gw_id, "modbus_tcp", true, gw_config);

        let dev_id = "dev-1";
        let device_config = &json!({"unit_id":1}).to_string();
        insert_device(&conn, dev_id, gw_id, true, device_config);

        let op_q1 = "op-q1";
        let op_config_q1 =
            &json!({"function":3,"address":0,"quantity":1,"poll_interval_ms":1000}).to_string();
        insert_operation(&conn, op_q1, dev_id, true, op_config_q1);

        let op_q125 = "op-q125";
        let op_config_q125 =
            &json!({"function":3,"address":0,"quantity":125,"poll_interval_ms":1000}).to_string();
        insert_operation(&conn, op_q125, dev_id, true, op_config_q125);

        let op_addr = "op-addr";
        let op_config_addr =
            &json!({"function":3,"address":65535,"quantity":1,"poll_interval_ms":1000}).to_string();
        insert_operation(&conn, op_addr, dev_id, true, op_config_addr);

        let op_poll = "op-poll";
        let op_config_poll =
            &json!({"function":3,"address":0,"quantity":2,"poll_interval_ms":100}).to_string();
        insert_operation(&conn, op_poll, dev_id, true, op_config_poll);

        let (cmd_result, skipped) =
            prepare_modbus_tcp(&conn, gw_id).expect("valid modbus tcp command");

        assert!(skipped.is_empty());
        assert_eq!(cmd_result.operations.len(), 4);
    }

    #[test]
    fn returns_not_found_for_unknown_gateway() {
        let conn = setup_db();
        let result = prepare_modbus_tcp(&conn, "non-existent-id");

        match result {
            Err(PrepareError::NotFound { table, key }) => {
                assert_eq!(table, "vgateway");
                assert_eq!(key, "non-existent-id");
            }
            other => panic!("Expected NotFound error, got {:?}", other),
        }
    }

    #[test]
    fn returns_disabled_for_disabled_gateway() {
        let conn = setup_db();
        let gw_id = "disabled-gw";
        let gw_config =
            &json!({ "host": "127.0.0.1", "port": 10502, "request_timeout_ms": 1500 }).to_string();
        insert_gateway(&conn, gw_id, "modbus_tcp", false, gw_config);

        let result = prepare_modbus_tcp(&conn, gw_id);

        match result {
            Err(PrepareError::Disabled { table, id }) => {
                assert_eq!(table, "vgateway");
                assert_eq!(id, "disabled-gw");
            }
            other => panic!("Expected Disabled error, got {:?}", other),
        }
    }
    #[test]
    fn rejects_gateway_with_wrong_driver_or_invalid_config() {
        let conn = setup_db();

        let cases = [
            (
                "unsupported-driver-gw",
                "modbus_rtu",
                json!({ "host": "127.0.0.1", "port": 10502, "request_timeout_ms": 1500 }),
                "driver",
            ),
            (
                "invalid-host-gw",
                "modbus_tcp",
                json!({ "host": "xyz", "port": 10502, "request_timeout_ms": 1500 }),
                "IP address",
            ),
            (
                "zero-timeout-gw",
                "modbus_tcp",
                json!({ "host": "127.0.0.1", "port": 10502, "request_timeout_ms": 0 }),
                "request_timeout_ms",
            ),
        ];

        for (gw_id, driver, config, expected_keyword) in cases {
            insert_gateway(&conn, gw_id, driver, true, &config.to_string());

            match prepare_modbus_tcp(&conn, gw_id) {
                Err(PrepareError::InvalidConfig { table, id, reason }) => {
                    assert_eq!(table, "vgateway");
                    assert_eq!(id, gw_id);
                    assert!(
                        reason.contains(expected_keyword),
                        "{gw_id}: expected reason to mention {expected_keyword:?}, got {reason:?}"
                    );
                }
                other => panic!("{gw_id}: expected InvalidConfig error, got {other:?}"),
            }
        }
    }
}
