-- SQLite schema version 1.
-- UUIDs, JSON objects and canonical UTC timestamps are stored as TEXT.
-- Execute PRAGMA foreign_keys = ON on EVERY application connection.
-- Names should be trimmed by the control plane before saving.
-- The control plane validates driver-specific config and updates updated_at
-- on normal application edits. The example.db correction preserves timestamps.
-- Config ownership and validation (application rules, not extra SQL constraints):
--   vgateway.config: shared transport/channel/session settings; no poll_interval_ms.
--   device.config: target/addressing and connection capabilities/profiles;
--                  no poll_interval_ms and no rpi_ms in connection profiles.
--   edge_operation.config: acquisition method, addressing and operation timing.
-- Resolve the driver through edge_operation.device_id -> device.gateway_id
-- -> vgateway.driver, then validate the driver and operation-specific JSON shape.
-- Validate disabled records too. Poll intervals are JSON integers greater than 0.
-- For polling Modbus reads (TCP/RTU/ASCII), require operation poll_interval_ms.
-- For scheduled EtherNet/IP explicit reads, require operation poll_interval_ms.
-- EtherNet/IP subscribe_input_only/subscribe_listen_only use operation rpi_ms
-- greater than 0, without poll_interval_ms; the operation owns the requested RPI.
-- OPC UA polling read uses operation poll_interval_ms; subscriptions instead use
-- sampling_interval_ms and publishing_interval_ms (event sampling interval is 0).
-- Fixed-window OPC UA history_read does not imply polling; preserve its history
-- parameters, including processing_interval_ms for processed history.
-- Do not require poll_interval_ms universally for every driver/operation.
-- example.db contains 4 gateways, 5 devices and 27 operations: 18 polling reads
-- with poll_interval_ms=1000 (8 TCP, 4 RTU, 5 EtherNet/IP, 1 OPC UA),
-- and 2 EtherNet/IP subscriptions with operation rpi_ms=100.
-- ASCII and PROFINET remain allowed drivers but have no rows in this example.
-- Table definitions, indexes, schema version, UUIDs and relationships are unchanged.
-- unit_id is optional here; when present it must be a JSON integer 0..255.
-- The control plane requires unit_id for Modbus and limits serial RTU/ASCII
-- read targets to 1..247. TCP targets may use 0..255.

PRAGMA foreign_keys = ON;

BEGIN TRANSACTION;

CREATE TABLE vgateway (
    id TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL,
    driver TEXT NOT NULL,
    config TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,

    CONSTRAINT ck_vgateway_name_not_blank
        CHECK (length(trim(name)) > 0),
    CONSTRAINT ck_vgateway_driver
        CHECK (driver IN ('modbus_tcp', 'modbus_rtu', 'modbus_ascii',
                          'opc_ua', 'ethernet_ip', 'profinet')),
    CONSTRAINT ck_vgateway_config_object
        CHECK (CASE WHEN json_valid(config)
                    THEN json_type(config) = 'object' ELSE 0 END),
    CONSTRAINT ck_vgateway_enabled
        CHECK (enabled IN (0, 1)),
    CONSTRAINT ck_vgateway_created_at
        CHECK (strftime('%Y-%m-%dT%H:%M:%SZ', created_at, '+0 seconds') IS NOT NULL
           AND created_at = strftime('%Y-%m-%dT%H:%M:%SZ', created_at, '+0 seconds')),
    CONSTRAINT ck_vgateway_updated_at
        CHECK (strftime('%Y-%m-%dT%H:%M:%SZ', updated_at, '+0 seconds') IS NOT NULL
           AND updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', updated_at, '+0 seconds')),
    CONSTRAINT ck_vgateway_timestamp_order
        CHECK (updated_at >= created_at)
);

CREATE UNIQUE INDEX ux_vgateway_name
    ON vgateway (name COLLATE NOCASE);

CREATE TABLE device (
    id TEXT NOT NULL PRIMARY KEY,
    gateway_id TEXT NOT NULL,
    name TEXT NOT NULL,
    slug TEXT NOT NULL,
    config TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,

    CONSTRAINT fk_device_gateway
        FOREIGN KEY (gateway_id) REFERENCES vgateway(id) ON DELETE RESTRICT,
    CONSTRAINT ck_device_name_not_blank
        CHECK (length(trim(name)) > 0),
    CONSTRAINT ck_device_slug
        CHECK (slug <> ''
           AND slug NOT GLOB '*[^a-z0-9-]*'
           AND slug NOT GLOB '-*'
           AND slug NOT GLOB '*-'
           AND slug NOT GLOB '*--*'),
    CONSTRAINT ck_device_config_object
        CHECK (CASE WHEN json_valid(config)
                    THEN json_type(config) = 'object' ELSE 0 END),
    CONSTRAINT ck_device_unit_id
        CHECK (CASE WHEN json_valid(config) THEN
                   CASE
                       WHEN json_type(config, '$.unit_id') IS NULL THEN 1
                       WHEN json_type(config, '$.unit_id') = 'integer'
                           THEN json_extract(config, '$.unit_id') BETWEEN 0 AND 255
                       ELSE 0
                   END
               ELSE 0 END),
    CONSTRAINT ck_device_enabled
        CHECK (enabled IN (0, 1)),
    CONSTRAINT ck_device_created_at
        CHECK (strftime('%Y-%m-%dT%H:%M:%SZ', created_at, '+0 seconds') IS NOT NULL
           AND created_at = strftime('%Y-%m-%dT%H:%M:%SZ', created_at, '+0 seconds')),
    CONSTRAINT ck_device_updated_at
        CHECK (strftime('%Y-%m-%dT%H:%M:%SZ', updated_at, '+0 seconds') IS NOT NULL
           AND updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', updated_at, '+0 seconds')),
    CONSTRAINT ck_device_timestamp_order
        CHECK (updated_at >= created_at)
);

CREATE UNIQUE INDEX ux_device_slug
    ON device (gateway_id, slug);
CREATE UNIQUE INDEX ux_device_name
    ON device (gateway_id, name COLLATE NOCASE);
CREATE UNIQUE INDEX ux_device_unit_id
    ON device (gateway_id, json_extract(config, '$.unit_id'));

CREATE TABLE edge_operation (
    id TEXT NOT NULL PRIMARY KEY,
    device_id TEXT NOT NULL,
    name TEXT NOT NULL,
    config TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,

    CONSTRAINT fk_edge_operation_device
        FOREIGN KEY (device_id) REFERENCES device(id) ON DELETE RESTRICT,
    CONSTRAINT ck_edge_operation_name_not_blank
        CHECK (length(trim(name)) > 0),
    CONSTRAINT ck_edge_operation_config_object
        CHECK (CASE WHEN json_valid(config)
                    THEN json_type(config) = 'object' ELSE 0 END),
    CONSTRAINT ck_edge_operation_enabled
        CHECK (enabled IN (0, 1)),
    CONSTRAINT ck_edge_operation_created_at
        CHECK (strftime('%Y-%m-%dT%H:%M:%SZ', created_at, '+0 seconds') IS NOT NULL
           AND created_at = strftime('%Y-%m-%dT%H:%M:%SZ', created_at, '+0 seconds')),
    CONSTRAINT ck_edge_operation_updated_at
        CHECK (strftime('%Y-%m-%dT%H:%M:%SZ', updated_at, '+0 seconds') IS NOT NULL
           AND updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', updated_at, '+0 seconds')),
    CONSTRAINT ck_edge_operation_timestamp_order
        CHECK (updated_at >= created_at)
);

CREATE UNIQUE INDEX ux_edge_operation_name
    ON edge_operation (device_id, name COLLATE NOCASE);

PRAGMA user_version = 1;

COMMIT;
