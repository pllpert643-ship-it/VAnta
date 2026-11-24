#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod admin_ui;
mod drivers;
mod instance;
mod maintenance;
mod platform;
mod policy;
mod pulse;
mod release;
mod scheduler;
mod services;
mod snapshots;
mod startup;
mod storage;
mod telemetry;
mod ui;

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn data_directory() -> Result<PathBuf, String> {
    if let Some(directory) = std::env::var_os("VANTA_NATIVE_DATA_DIR") {
        return Ok(PathBuf::from(directory));
    }
    directories::ProjectDirs::from(
        "",
        "",
        if release::STORE {
            "VAntaStore"
        } else {
            "VAntaNative"
        },
    )
    .map(|p| p.data_local_dir().to_path_buf())
    .ok_or("Cannot locate local application data directory".into())
}
fn command_line(args: &[String]) -> Result<bool, String> {
    match args.first().map(String::as_str) {
        None => Ok(false),
        Some("--help" | "-h") => {
            println!(
                "VAnta Native 0.2\nUsage: VAntaNative [--help | --version | --release-info | --diagnostics OUTPUT.json | --inventory OUTPUT.json | --smoke-test | --gpu-pulse MB SECONDS]\nNative GUI: run without arguments. VANTA_NATIVE_DATA_DIR selects isolated storage."
            );
            Ok(true)
        }
        Some("--release-info") if args.len() == 1 => {
            println!(
                "{}",
                serde_json::json!({"version": env!("CARGO_PKG_VERSION"), "edition": release::EDITION,
                "system_mutations": !release::STORE, "requires_elevation": false})
            );
            Ok(true)
        }
        Some("--version") => {
            println!("VAnta Native {}", env!("CARGO_PKG_VERSION"));
            Ok(true)
        }
        Some("--gpu-pulse") if args.len() == 3 => {
            let memory = args[1].parse().map_err(|_| "Invalid memory size")?;
            let duration = args[2].parse().map_err(|_| "Invalid duration")?;
            pulse::run(memory, duration)?;
            Ok(true)
        }
        Some("--diagnostics") if args.len() == 2 => {
            let worker = telemetry::TelemetryWorker::spawn();
            let deadline = Instant::now() + Duration::from_secs(15);
            let mut found = None;
            while Instant::now() < deadline {
                if let Some(snapshot) = worker.latest() {
                    found = Some(snapshot);
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let snapshot = found.ok_or("Telemetry timed out")?;
            snapshots::save(&PathBuf::from(&args[1]), &snapshot)?;
            println!(
                "Captured {} processes; {} logical CPUs; {} bytes RAM",
                snapshot.processes.len(),
                snapshot.cpu_count,
                snapshot.total_memory
            );
            Ok(true)
        }
        Some("--inventory") if args.len() == 2 => {
            let services = services::list()?.into_iter().map(|row| serde_json::json!({"name":row.name,"display_name":row.display_name,"state":row.state,"pid":row.pid})).collect::<Vec<_>>();
            let startup = startup::list()?.into_iter().map(|row| serde_json::json!({"scope":row.scope,"name":row.name,"command":row.command,"enabled":row.enabled,"writable":row.writable})).collect::<Vec<_>>();
            let bytes = serde_json::to_vec_pretty(&serde_json::json!({"services":services,"startup":startup,"elevated":maintenance::is_elevated()?})).map_err(|e| e.to_string())?;
            storage::atomic_write(&PathBuf::from(&args[1]), &bytes)?;
            println!("Read-only service and startup inventory exported.");
            Ok(true)
        }
        Some("--smoke-test") => {
            launch_gui(true)?;
            Ok(true)
        }
        _ => Err("Unrecognized arguments; use --help".into()),
    }
}
fn launch_gui(smoke: bool) -> Result<(), String> {
    let directory = data_directory()?;
    let _instance = instance::InstanceGuard::acquire(&directory)?;
    ui::launch(directory, smoke).map_err(|e| e.to_string())
}
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = (|| {
        if command_line(&args)? {
            return Ok(());
        }
        launch_gui(false)
    })();
    if let Err(error) = result {
        eprintln!("VAnta: {error}");
        if args.is_empty() {
            rfd::MessageDialog::new()
                .set_title("VAnta could not start")
                .set_description(&error)
                .set_level(rfd::MessageLevel::Error)
                .show();
        }
        std::process::exit(1);
    }
}

#[cfg(all(test, feature = "store"))]
mod store_tests {
    use super::*;
    #[test]
    fn store_backends_refuse_system_mutations_even_when_called_without_ui() {
        let entry = startup::StartupEntry {
            scope: "HKCU Run (64-bit)".into(),
            name: "VAnta test - never write".into(),
            command: "no-such-app.exe".into(),
            enabled: Some(true),
            writable: true,
        };
        // All targets are deliberately invalid. This test must never request admin
        // rights or mutate a real system even if the release gate regresses.
        assert!(
            startup::set_enabled(&entry, false)
                .unwrap_err()
                .contains("Store edition")
        );
        assert!(
            services::control(
                "VAnta-test-service-does-not-exist",
                services::ServiceAction::Stop
            )
            .unwrap_err()
            .contains("Store edition")
        );
        assert!(
            maintenance::execute(maintenance::Operation::PurgeStandby)
                .unwrap_err()
                .contains("Store edition")
        );
    }
}
