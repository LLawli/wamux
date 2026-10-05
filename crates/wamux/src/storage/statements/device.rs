//! `DeviceStore`: the whole `Device` as one protobuf blob per `device_id`.

pub const SAVE_DEVICE: &str = "INSERT INTO device (device_id, data) VALUES ($1, $2)
     ON CONFLICT (device_id) DO UPDATE SET data = EXCLUDED.data";

pub const LOAD_DEVICE: &str = "SELECT data FROM device WHERE device_id = $1";

pub const DEVICE_EXISTS: &str = "SELECT EXISTS(SELECT 1 FROM device WHERE device_id = $1)";

pub const CREATE_DEVICE: &str = "INSERT INTO device (device_id, data) VALUES ($1, $2)
     ON CONFLICT (device_id) DO NOTHING";
