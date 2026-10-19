//! Release 1 local Worker process.
//!
//! The Worker owns periodic schedule staging and JobStore dispatch. Its
//! private Unix socket is intentionally local-only. It does not expose a
//! network or remote-Worker protocol.

use qadren_core::job_execution::{
    run_local_scenario_file_copy, run_local_scenario_file_replace, LocalFileCopyError,
    LocalFileCopyStatus,
};
use qadren_core::jobs::JobStore;
use qadren_core::registry::Registry;
use qadren_core::schedule_dispatch::dispatch_pending_schedule_jobs;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime};

const POLL_INTERVAL: Duration = Duration::from_secs(60);
const POLL_OVERLAP: Duration = Duration::from_secs(90);

#[derive(Debug)]
struct WorkerPaths {
    socket: PathBuf,
    registry: PathBuf,
    jobs: PathBuf,
}

#[derive(Debug)]
struct WorkerLock {
    _file: File,
}

impl WorkerLock {
    fn acquire(socket: &Path) -> Result<Self, String> {
        let parent = socket
            .parent()
            .ok_or_else(|| "worker_socket_parent_missing".to_string())?;
        let parent_existed = parent.exists();
        fs::create_dir_all(parent)
            .map_err(|_| "worker_socket_directory_unavailable".to_string())?;
        if !parent_existed {
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
                .map_err(|_| "worker_socket_directory_unavailable".to_string())?;
        }
        let parent_metadata = fs::symlink_metadata(parent)
            .map_err(|_| "worker_socket_directory_unavailable".to_string())?;
        if !parent_metadata.file_type().is_dir()
            || parent_metadata.file_type().is_symlink()
            || parent_metadata.permissions().mode() & 0o077 != 0
        {
            return Err("worker_socket_directory_unsafe".to_string());
        }
        let lock_path = parent.join("worker.lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lock_path)
            .map_err(|_| "worker_lock_unavailable".to_string())?;
        let result = unsafe {
            libc::flock(
                std::os::unix::io::AsRawFd::as_raw_fd(&file),
                libc::LOCK_EX | libc::LOCK_NB,
            )
        };
        if result != 0 {
            return Err("worker_already_running".to_string());
        }
        Ok(Self { _file: file })
    }
}

#[derive(Debug, Deserialize)]
struct Request {
    command: String,
    #[serde(default)]
    scenario_id: Option<String>,
    #[serde(default)]
    scenario_revision: Option<u64>,
    #[serde(default)]
    source_root: Option<String>,
    #[serde(default)]
    source_connection_id: Option<String>,
    #[serde(default)]
    source_relative_path: Option<String>,
    #[serde(default)]
    destination_root: Option<String>,
    #[serde(default)]
    destination_connection_id: Option<String>,
    #[serde(default)]
    destination_relative_path: Option<String>,
    #[serde(default)]
    job_id: Option<String>,
    #[serde(default)]
    replacement_consent: Option<bool>,
}

#[derive(Debug, Serialize)]
struct Response<'a> {
    ok: bool,
    status: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    staged: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dispatched: Option<usize>,
}

fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::<chrono::Utc>::from(SystemTime::now())
}

fn dispatch_due(paths: &WorkerPaths) -> Result<(usize, usize), String> {
    let through = now();
    let after = through
        - chrono::Duration::from_std(POLL_OVERLAP)
            .map_err(|_| "worker_clock_unavailable".to_string())?;
    let registry =
        Registry::open(&paths.registry).map_err(|_| "registry_unavailable".to_string())?;
    let staged = registry
        .stage_due_schedule_occurrences(after, through)
        .map_err(|_| "schedule_stage_unavailable".to_string())?;
    let mut jobs = JobStore::open(&paths.jobs).map_err(|_| "job_store_unavailable".to_string())?;
    let dispatched = dispatch_pending_schedule_jobs(&registry, &mut jobs)
        .map_err(|_| "schedule_dispatch_unavailable".to_string())?;
    Ok((staged, dispatched))
}

fn remove_stale_socket(socket: &Path) -> Result<(), String> {
    match fs::symlink_metadata(socket) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            fs::remove_file(socket).map_err(|_| "worker_socket_unavailable".to_string())
        }
        Ok(_) => Err("worker_socket_path_unsafe".to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("worker_socket_unavailable".to_string()),
    }
}

fn execute_create(paths: &WorkerPaths, request: &Request) -> Result<&'static str, &'static str> {
    let (
        Some(scenario_id),
        Some(scenario_revision),
        Some(source_root),
        Some(source_connection_id),
        Some(source_relative_path),
        Some(destination_root),
        Some(destination_connection_id),
        Some(destination_relative_path),
        Some(job_id),
    ) = (
        request.scenario_id.as_deref(),
        request.scenario_revision,
        request.source_root.as_deref(),
        request.source_connection_id.as_deref(),
        request.source_relative_path.as_deref(),
        request.destination_root.as_deref(),
        request.destination_connection_id.as_deref(),
        request.destination_relative_path.as_deref(),
        request.job_id.as_deref(),
    )
    else {
        return Err("invalid_request");
    };
    match run_local_scenario_file_copy(
        &paths.jobs,
        &paths.registry,
        scenario_id,
        scenario_revision,
        Path::new(source_root),
        source_connection_id,
        Path::new(source_relative_path),
        Path::new(destination_root),
        destination_connection_id,
        Path::new(destination_relative_path),
        job_id,
    ) {
        Ok(LocalFileCopyStatus::Completed) => Ok("completed"),
        Ok(LocalFileCopyStatus::Existing(_)) => Ok("existing"),
        Err(LocalFileCopyError::TargetExists) => Err("target_exists"),
        Err(LocalFileCopyError::ConnectionUnavailable) => Err("local_connection_unavailable"),
        Err(LocalFileCopyError::ScenarioUnavailable) => Err("scenario_unavailable"),
        Err(LocalFileCopyError::ScenarioScopeMismatch) => Err("scenario_scope_mismatch"),
        Err(LocalFileCopyError::CapabilityUnavailable) => Err("local_capability_unavailable"),
        Err(LocalFileCopyError::VerificationFailed) => Err("verification_failed"),
        Err(LocalFileCopyError::OutcomeUnknown(_)) => Err("outcome_unknown"),
        Err(LocalFileCopyError::InvalidRequest) => Err("invalid_request"),
        Err(LocalFileCopyError::Local(_)) => Err("local_operation_failed"),
        Err(LocalFileCopyError::Store(_)) => Err("job_store_unavailable"),
        Err(_) => Err("scenario_create_unavailable"),
    }
}

fn execute_replace(paths: &WorkerPaths, request: &Request) -> Result<&'static str, &'static str> {
    let (
        Some(scenario_id),
        Some(scenario_revision),
        Some(source_root),
        Some(source_connection_id),
        Some(source_relative_path),
        Some(destination_root),
        Some(destination_connection_id),
        Some(destination_relative_path),
        Some(job_id),
    ) = (
        request.scenario_id.as_deref(),
        request.scenario_revision,
        request.source_root.as_deref(),
        request.source_connection_id.as_deref(),
        request.source_relative_path.as_deref(),
        request.destination_root.as_deref(),
        request.destination_connection_id.as_deref(),
        request.destination_relative_path.as_deref(),
        request.job_id.as_deref(),
    )
    else {
        return Err("invalid_request");
    };
    match run_local_scenario_file_replace(
        &paths.jobs,
        &paths.registry,
        scenario_id,
        scenario_revision,
        Path::new(source_root),
        source_connection_id,
        Path::new(source_relative_path),
        Path::new(destination_root),
        destination_connection_id,
        Path::new(destination_relative_path),
        job_id,
        request.replacement_consent.unwrap_or(false),
    ) {
        Ok(LocalFileCopyStatus::Completed) => Ok("completed"),
        Ok(LocalFileCopyStatus::Existing(_)) => Ok("existing"),
        Err(LocalFileCopyError::ConsentRequired) => Err("consent_required"),
        Err(LocalFileCopyError::TargetExists) => Err("target_exists"),
        Err(LocalFileCopyError::ConnectionUnavailable) => Err("local_connection_unavailable"),
        Err(LocalFileCopyError::ScenarioUnavailable) => Err("scenario_unavailable"),
        Err(LocalFileCopyError::ScenarioScopeMismatch) => Err("scenario_scope_mismatch"),
        Err(LocalFileCopyError::CapabilityUnavailable) => Err("local_capability_unavailable"),
        Err(LocalFileCopyError::VerificationFailed) => Err("verification_failed"),
        Err(LocalFileCopyError::OutcomeUnknown(_)) => Err("outcome_unknown"),
        Err(LocalFileCopyError::InvalidRequest) => Err("invalid_request"),
        Err(LocalFileCopyError::Local(_)) => Err("local_operation_failed"),
        Err(LocalFileCopyError::Store(_)) => Err("job_store_unavailable"),
        Err(_) => Err("scenario_replace_unavailable"),
    }
}

fn respond(stream: &mut UnixStream, response: &Response<'_>) {
    if let Ok(json) = serde_json::to_string(response) {
        let _ = writeln!(stream, "{json}");
    }
}

fn handle_client(mut stream: UnixStream, paths: &WorkerPaths, stop: &mut bool) {
    let request = {
        let mut line = String::new();
        let mut reader = BufReader::new(&stream);
        if reader.read_line(&mut line).is_err() {
            return;
        }
        serde_json::from_str::<Request>(&line).ok()
    };
    let Some(request) = request else {
        respond(
            &mut stream,
            &Response {
                ok: false,
                status: "invalid_request",
                staged: None,
                dispatched: None,
            },
        );
        return;
    };
    match request.command.as_str() {
        "status" => respond(
            &mut stream,
            &Response {
                ok: true,
                status: "running",
                staged: None,
                dispatched: None,
            },
        ),
        "poll" => match dispatch_due(paths) {
            Ok((staged, dispatched)) => respond(
                &mut stream,
                &Response {
                    ok: true,
                    status: "running",
                    staged: Some(staged),
                    dispatched: Some(dispatched),
                },
            ),
            Err(status) => respond(
                &mut stream,
                &Response {
                    ok: false,
                    status: &status,
                    staged: None,
                    dispatched: None,
                },
            ),
        },
        "execute_create" => match execute_create(paths, &request) {
            Ok(status) => respond(
                &mut stream,
                &Response {
                    ok: true,
                    status,
                    staged: None,
                    dispatched: None,
                },
            ),
            Err(status) => respond(
                &mut stream,
                &Response {
                    ok: false,
                    status,
                    staged: None,
                    dispatched: None,
                },
            ),
        },
        "execute_replace" => match execute_replace(paths, &request) {
            Ok(status) => respond(
                &mut stream,
                &Response {
                    ok: true,
                    status,
                    staged: None,
                    dispatched: None,
                },
            ),
            Err(status) => respond(
                &mut stream,
                &Response {
                    ok: false,
                    status,
                    staged: None,
                    dispatched: None,
                },
            ),
        },
        "stop" => {
            *stop = true;
            respond(
                &mut stream,
                &Response {
                    ok: true,
                    status: "stopping",
                    staged: None,
                    dispatched: None,
                },
            );
        }
        _ => respond(
            &mut stream,
            &Response {
                ok: false,
                status: "unsupported_command",
                staged: None,
                dispatched: None,
            },
        ),
    }
}

fn run(paths: WorkerPaths, once: bool) -> Result<(), String> {
    let _lock = WorkerLock::acquire(&paths.socket)?;
    if once {
        dispatch_due(&paths)?;
        return Ok(());
    }
    remove_stale_socket(&paths.socket)?;
    let listener =
        UnixListener::bind(&paths.socket).map_err(|_| "worker_socket_unavailable".to_string())?;
    fs::set_permissions(&paths.socket, fs::Permissions::from_mode(0o600))
        .map_err(|_| "worker_socket_unavailable".to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|_| "worker_socket_unavailable".to_string())?;
    let mut stop = false;
    let mut next_poll = SystemTime::now();
    while !stop {
        if SystemTime::now() >= next_poll {
            let _ = dispatch_due(&paths);
            next_poll = SystemTime::now() + POLL_INTERVAL;
        }
        match listener.accept() {
            Ok((stream, _)) => handle_client(stream, &paths, &mut stop),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50))
            }
            Err(_) => return Err("worker_socket_unavailable".to_string()),
        }
    }
    let _ = fs::remove_file(&paths.socket);
    Ok(())
}

fn usage() -> ! {
    eprintln!("usage: qadren-worker [--socket PATH --registry PATH --jobs PATH] [--once]");
    std::process::exit(64);
}

fn parse_paths() -> (WorkerPaths, bool) {
    let mut socket = None;
    let mut registry = None;
    let mut jobs = None;
    let mut once = false;
    let mut args = env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--socket" => socket = args.next().map(PathBuf::from),
            "--registry" => registry = args.next().map(PathBuf::from),
            "--jobs" => jobs = args.next().map(PathBuf::from),
            "--once" => once = true,
            _ => usage(),
        }
    }
    match (socket, registry, jobs) {
        (None, None, None) => {
            let home = env::var_os("HOME").unwrap_or_else(|| usage());
            let base = PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("QADREN");
            (
                WorkerPaths {
                    socket: base.join("worker.sock"),
                    registry: base.join("connections.sqlite3"),
                    jobs: base.join("jobs.sqlite3"),
                },
                once,
            )
        }
        (Some(socket), Some(registry), Some(jobs)) => (
            WorkerPaths {
                socket,
                registry,
                jobs,
            },
            once,
        ),
        _ => usage(),
    }
}

fn main() {
    let (paths, once) = parse_paths();
    if let Err(error) = run(paths, once) {
        eprintln!("qadren-worker: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn root(name: &str) -> PathBuf {
        let path = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!(
                "qadren-worker-{name}-{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir_all(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[test]
    fn duplicate_worker_lock_is_denied_before_store_access() {
        let root = root("lock");
        let socket = root.join("worker.sock");
        let first = WorkerLock::acquire(&socket).unwrap();
        assert_eq!(
            WorkerLock::acquire(&socket).unwrap_err(),
            "worker_already_running"
        );
        drop(first);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn one_poll_stages_and_dispatches_a_due_scenario_job() {
        let root = root("poll");
        fs::create_dir_all(&root).unwrap();
        let paths = WorkerPaths {
            socket: root.join("worker.sock"),
            registry: root.join("registry.sqlite3"),
            jobs: root.join("jobs.sqlite3"),
        };
        let registry = Registry::open(&paths.registry).unwrap();
        let left = registry.add("local_filesystem", "Left").unwrap();
        let right = registry.add("local_filesystem", "Right").unwrap();
        let scenario = registry
            .create_scenario("Documents", &left, "/", &right, "/archive", "left_to_right")
            .unwrap();
        let current = now();
        registry
            .create_scenario_schedule(
                &scenario,
                "UTC",
                current.weekday().num_days_from_monday() as u8,
                current.hour() as u8,
                current.minute() as u8,
            )
            .unwrap();
        let (staged, dispatched) = dispatch_due(&paths).unwrap();
        assert_eq!((staged, dispatched), (1, 1));
        assert_eq!(
            JobStore::open(&paths.jobs)
                .unwrap()
                .list_jobs()
                .unwrap()
                .len(),
            1
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn private_socket_reports_status_and_stops_the_worker() {
        let root = root("socket");
        fs::create_dir_all(&root).unwrap();
        let paths = WorkerPaths {
            socket: root.join("worker.sock"),
            registry: root.join("registry.sqlite3"),
            jobs: root.join("jobs.sqlite3"),
        };
        let socket = paths.socket.clone();
        let handle = thread::spawn(move || run(paths, false));
        for _ in 0..100 {
            if socket.exists() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(socket.exists());

        let mut status = UnixStream::connect(&socket).unwrap();
        writeln!(status, r#"{{"command":"status"}}"#).unwrap();
        let mut response = String::new();
        BufReader::new(status).read_line(&mut response).unwrap();
        assert!(response.contains(r#""status":"running"#));

        let mut stop = UnixStream::connect(&socket).unwrap();
        writeln!(stop, r#"{{"command":"stop"}}"#).unwrap();
        response.clear();
        BufReader::new(stop).read_line(&mut response).unwrap();
        assert!(response.contains(r#""status":"stopping"#));
        assert!(handle.join().unwrap().is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_non_socket_at_the_socket_path_is_not_deleted() {
        let root = root("unsafe-socket");
        fs::create_dir_all(&root).unwrap();
        let socket = root.join("worker.sock");
        fs::write(&socket, b"not a socket").unwrap();
        assert_eq!(
            remove_stale_socket(&socket).unwrap_err(),
            "worker_socket_path_unsafe"
        );
        assert_eq!(fs::read(&socket).unwrap(), b"not a socket");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn private_socket_executes_pinned_scenario_create_and_confirmed_replace() {
        let root = root("scenario-create");
        let source_root = root.join("source");
        let destination_root = root.join("destination");
        let socket_directory = std::env::temp_dir().join(format!(
            "qw-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&socket_directory).unwrap();
        fs::set_permissions(&socket_directory, fs::Permissions::from_mode(0o700)).unwrap();
        let socket = socket_directory.join("worker.sock");
        fs::create_dir_all(&source_root).unwrap();
        fs::create_dir_all(&destination_root).unwrap();
        fs::write(source_root.join("report.txt"), b"worker content").unwrap();
        let paths = WorkerPaths {
            socket,
            registry: root.join("registry.sqlite3"),
            jobs: root.join("jobs.sqlite3"),
        };
        let registry = Registry::open(&paths.registry).unwrap();
        let left = registry.add("local_filesystem", "Left").unwrap();
        let right = registry.add("local_filesystem", "Right").unwrap();
        let scenario = registry
            .create_scenario(
                "Documents",
                &left,
                source_root.to_str().unwrap(),
                &right,
                destination_root.to_str().unwrap(),
                "left_to_right",
            )
            .unwrap();
        drop(registry);
        let socket = paths.socket.clone();
        let handle = thread::spawn(move || run(paths, false));
        for _ in 0..100 {
            if socket.exists() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let request = serde_json::json!({
            "command":"execute_create",
            "scenario_id":scenario,
            "scenario_revision":1,
            "source_root":source_root,
            "source_connection_id":left,
            "source_relative_path":"report.txt",
            "destination_root":destination_root,
            "destination_connection_id":right,
            "destination_relative_path":"report.txt",
            "job_id":"worker-scenario-create"
        });
        let mut stream = UnixStream::connect(&socket).unwrap();
        writeln!(stream, "{request}").unwrap();
        let mut response = String::new();
        BufReader::new(stream).read_line(&mut response).unwrap();
        assert!(
            response.contains(r#""status":"completed"#),
            "unexpected Worker response: {response}"
        );
        assert_eq!(
            fs::read(destination_root.join("report.txt")).unwrap(),
            b"worker content"
        );
        fs::write(source_root.join("report.txt"), b"replacement content").unwrap();
        let declined_replace = serde_json::json!({
            "command":"execute_replace",
            "scenario_id":scenario,
            "scenario_revision":1,
            "source_root":source_root,
            "source_connection_id":left,
            "source_relative_path":"report.txt",
            "destination_root":destination_root,
            "destination_connection_id":right,
            "destination_relative_path":"report.txt",
            "job_id":"worker-scenario-replace-declined",
            "replacement_consent":false
        });
        let mut stream = UnixStream::connect(&socket).unwrap();
        writeln!(stream, "{declined_replace}").unwrap();
        response.clear();
        BufReader::new(stream).read_line(&mut response).unwrap();
        assert!(
            response.contains(r#""status":"consent_required""#),
            "unexpected declined replacement response: {response}"
        );
        assert_eq!(
            fs::read(destination_root.join("report.txt")).unwrap(),
            b"worker content"
        );
        let confirmed_replace = serde_json::json!({
            "command":"execute_replace",
            "scenario_id":scenario,
            "scenario_revision":1,
            "source_root":source_root,
            "source_connection_id":left,
            "source_relative_path":"report.txt",
            "destination_root":destination_root,
            "destination_connection_id":right,
            "destination_relative_path":"report.txt",
            "job_id":"worker-scenario-replace-confirmed",
            "replacement_consent":true
        });
        let mut stream = UnixStream::connect(&socket).unwrap();
        writeln!(stream, "{confirmed_replace}").unwrap();
        response.clear();
        BufReader::new(stream).read_line(&mut response).unwrap();
        assert!(
            response.contains(r#""status":"completed""#),
            "unexpected confirmed replacement response: {response}"
        );
        assert_eq!(
            fs::read(destination_root.join("report.txt")).unwrap(),
            b"replacement content"
        );
        assert_eq!(
            JobStore::open(&root.join("jobs.sqlite3"))
                .unwrap()
                .list_jobs()
                .unwrap()
                .len(),
            2
        );
        let mut stop = UnixStream::connect(&socket).unwrap();
        writeln!(stop, r#"{{"command":"stop"}}"#).unwrap();
        assert!(handle.join().unwrap().is_ok());
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(socket_directory).unwrap();
    }
}
