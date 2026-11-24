use crate::{storage::atomic_write, telemetry::Snapshot};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs::File, io::Read, path::Path};
const MAX_BYTES: u64 = 32 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub schema_version: u32,
    pub source: String,
    pub snapshot: Snapshot,
}

pub fn save(path: &Path, snapshot: &Snapshot) -> Result<(), String> {
    validate(snapshot)?;
    let document = Document {
        schema_version: 1,
        source: "VAnta Native; observed counters, not a hardware diagnosis".into(),
        snapshot: snapshot.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&document).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Snapshot exceeds 32 MiB limit".into());
    }
    atomic_write(path, &bytes)
}
pub fn load(path: &Path) -> Result<Snapshot, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Snapshot exceeds 32 MiB limit".into());
    }
    let document: Document =
        serde_json::from_slice(&bytes).map_err(|e| format!("Invalid native snapshot: {e}"))?;
    if document.schema_version != 1 {
        return Err(
            "Unsupported snapshot schema; use a VAnta Native version-1 JSON snapshot".into(),
        );
    }
    validate(&document.snapshot)?;
    Ok(document.snapshot)
}
pub fn validate(snapshot: &Snapshot) -> Result<(), String> {
    if snapshot.unix_seconds == 0
        || snapshot.total_memory == 0
        || snapshot.cpu_count == 0
        || snapshot.cpu_count > 65536
    {
        return Err("Invalid timestamp, RAM capacity or logical CPU count".into());
    }
    if !snapshot.cpu_percent.is_finite()
        || !(0.0..=100.0).contains(&snapshot.cpu_percent)
        || snapshot.used_memory > snapshot.total_memory
        || snapshot.available_memory > snapshot.total_memory
        || snapshot.used_swap > snapshot.total_swap
    {
        return Err("Snapshot counters are outside valid ranges".into());
    }
    if snapshot.processes.len() > 100_000 {
        return Err("Too many process records".into());
    }
    let mut identities = HashSet::new();
    for p in &snapshot.processes {
        if !p.cpu_percent.is_finite()
            || !(0.0..=100.1).contains(&p.cpu_percent)
            || p.name.len() > 32768
            || p.executable.as_ref().is_some_and(|s| s.len() > 32768)
            || !identities.insert(p.pid)
        {
            return Err("Invalid or duplicate process record".into());
        }
    }
    if let Some(gpu) = &snapshot.gpu
        && (gpu.total_bytes == 0
            || gpu.used_bytes > gpu.total_bytes
            || gpu.utilization_percent.is_some_and(|x| x > 100)
            || gpu.power_w.is_some_and(|x| !x.is_finite() || x < 0.0))
    {
        return Err("Invalid GPU counters".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Snapshot {
        serde_json::from_value(serde_json::json!({"unix_seconds":1794900000_u64,"cpu_percent":12.0,"cpu_count":8,"used_memory":4096,"total_memory":8192,"available_memory":4096,"used_swap":0,"total_swap":0,"processes":[{"pid":50,"name":"example.exe","cpu_percent":2.5,"memory_bytes":1024,"read_bytes":32,"written_bytes":16,"start_time":1794800000_u64,"executable":null}],"gpu":null,"gpu_error":"Unavailable","collection_ms":20})).unwrap()
    }
    fn directory() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "vanta-snapshot-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }
    #[test]
    fn saved_native_snapshot_round_trips_and_replaces_previous_document() {
        let dir = directory();
        let path = dir.join("snapshot.json");
        let mut snapshot = fixture();
        save(&path, &snapshot).unwrap();
        snapshot.cpu_percent = 38.0;
        save(&path, &snapshot).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.cpu_percent, 38.0);
        assert_eq!(loaded.processes[0].name, "example.exe");
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn invalid_export_does_not_destroy_the_previous_snapshot() {
        let dir = directory();
        let path = dir.join("snapshot.json");
        let mut snapshot = fixture();
        save(&path, &snapshot).unwrap();
        snapshot.cpu_percent = f32::NAN;
        assert!(save(&path, &snapshot).is_err());
        assert_eq!(load(&path).unwrap().cpu_percent, 12.0);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn duplicate_pid_and_out_of_range_gpu_counters_are_rejected() {
        let mut snapshot = fixture();
        snapshot.processes.push(snapshot.processes[0].clone());
        assert!(validate(&snapshot).is_err());
        snapshot.processes.pop();
        snapshot.gpu = Some(crate::telemetry::GpuSnapshot {
            name: "GPU".into(),
            utilization_percent: Some(101),
            used_bytes: 1,
            total_bytes: 10,
            temperature_c: None,
            power_w: None,
        });
        assert!(validate(&snapshot).is_err());
    }
    #[test]
    fn unsupported_schemas_and_oversized_files_are_rejected_before_use() {
        let dir = directory();
        let path = dir.join("bad.json");
        let document = Document {
            schema_version: 2,
            source: "test".into(),
            snapshot: fixture(),
        };
        std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        assert!(load(&path).unwrap_err().contains("Unsupported"));
        std::fs::File::create(&path)
            .unwrap()
            .set_len(MAX_BYTES + 1)
            .unwrap();
        assert!(load(&path).unwrap_err().contains("32 MiB"));
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
