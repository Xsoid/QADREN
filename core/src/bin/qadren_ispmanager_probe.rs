//! Gate 0 read-only probe for an ISPmanager 6 panel (D-027, proposed).
//!
//! Usage: `<password source> | qadren-ispmanager-probe --url <panel URL> --login <login>`.
//! The password comes from standard input, never from an argument. The output
//! has result classes and field names only: no values, counts, or session ID.
//!
//! `--db-dump <database> --ssh-port <port> --host-key <SHA256 fingerprint>`
//! checks the panel dump `db.dump` of one database (D-027 section 7b.9)
//! instead of the lists. It lists the home folder and `/tmp` over SFTP
//! before and after the call, and prints only result classes and counts:
//! the response type, whether it is a complete dump, and how many files
//! appeared or changed on the server. It writes nothing on the server.
use qadren_core::ispmanager::{Category, IspError, ListOutcome, PanelUrl, Session};
use qadren_core::ispmanager_sftp::SftpConnection;
use std::collections::BTreeMap;
use std::io::{self, BufRead, IsTerminal, Write};
use std::process::ExitCode;

fn argument(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

/// Reads one line from standard input. On a terminal, echo is off.
fn read_password() -> io::Result<String> {
    let stdin = io::stdin();
    let terminal = stdin.is_terminal();
    let mut saved: Option<libc::termios> = None;
    if terminal {
        eprint!("Password: ");
        io::stderr().flush()?;
        // SAFETY: tcgetattr and tcsetattr get a valid descriptor and a
        // termios value that lives for the whole call.
        unsafe {
            let mut attrs: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut attrs) == 0 {
                saved = Some(attrs);
                attrs.c_lflag &= !libc::ECHO;
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &attrs);
            }
        }
    }
    let mut line = String::new();
    let result = stdin.lock().read_line(&mut line);
    if let Some(attrs) = saved {
        // SAFETY: restores the terminal state read above.
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &attrs);
        }
        eprintln!();
    }
    result?;
    Ok(line.trim_end_matches(['\r', '\n']).to_owned())
}

fn describe(result: &Result<ListOutcome, IspError>) -> String {
    match result {
        Ok(ListOutcome::Items { fields }) => format!("ok\t{}", fields.join(",")),
        Ok(ListOutcome::Empty) => "empty\t".to_owned(),
        Err(err) => format!("error\t{err}"),
    }
}

/// Files below `root` with their size and modification time, up to `depth`
/// folder levels and `limit` entries. The second value is true when the
/// walk stopped at a limit. An unreadable folder is skipped.
fn server_files(
    sftp: &SftpConnection,
    root: &str,
    depth: usize,
    limit: usize,
) -> (BTreeMap<String, (u64, u32)>, bool) {
    let mut files = BTreeMap::new();
    let mut folders = vec![(root.trim_end_matches('/').to_owned(), 0usize)];
    let mut truncated = false;
    while let Some((folder, level)) = folders.pop() {
        let Ok(entries) = sftp.read_dir(if folder.is_empty() { "/" } else { &folder }) else {
            continue;
        };
        for (name, meta) in entries {
            let path = format!("{folder}/{name}");
            if files.len() >= limit {
                truncated = true;
                break;
            }
            match meta.kind {
                "dir" if level + 1 < depth => folders.push((path, level + 1)),
                "dir" => truncated = true,
                "file" => {
                    files.insert(path, (meta.size, meta.modified.unwrap_or(0)));
                }
                _ => {}
            }
        }
    }
    (files, truncated)
}

/// The `db.dump` check. It prints classes and counts only.
fn check_db_dump(session: &mut Session, sftp: &SftpConnection, database: &str) -> ExitCode {
    let Ok(home) = sftp.home() else {
        println!("sftp\thome\terror");
        return ExitCode::FAILURE;
    };
    let roots = [(home.as_str(), 8usize), ("/tmp", 2), ("/var/tmp", 2)];
    let snapshot = || {
        roots
            .iter()
            .map(|(root, depth)| server_files(sftp, root, *depth, 200_000))
            .collect::<Vec<_>>()
    };
    let before = snapshot();
    let key = match qadren_core::ispmanager::database_key(session, database) {
        Ok(key) => key,
        Err(code) => {
            println!("db.dump\terror\t{code}");
            return ExitCode::FAILURE;
        }
    };
    let result = session.database_dump(&key, |_| true);
    // A server can finish a temporary file a moment after the response.
    std::thread::sleep(std::time::Duration::from_secs(5));
    let after = snapshot();
    match &result {
        Ok(_) => println!("db.dump\tcomplete"),
        Err(error) => println!("db.dump\terror\t{error:?}"),
    }
    for (((root, _), (old, old_cut)), (new, new_cut)) in roots.iter().zip(&before).zip(&after) {
        let label = if *root == home { "home" } else { root };
        let added: Vec<&String> = new.keys().filter(|path| !old.contains_key(*path)).collect();
        let changed = new
            .iter()
            .filter(|(path, value)| old.get(*path).is_some_and(|old| old != *value))
            .count();
        let removed = old.keys().filter(|path| !new.contains_key(*path)).count();
        let dumps = added
            .iter()
            .filter(|path| {
                [".sql", ".gz", ".sql.gz", ".dump"]
                    .iter()
                    .any(|end| path.ends_with(end))
            })
            .count();
        println!(
            "server\t{label}\tadded={}\tchanged={changed}\tremoved={removed}\tadded_dump_like={dumps}\tlisting_limited={}",
            added.len(),
            *old_cut || *new_cut,
        );
    }
    if result.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let (Some(url), Some(login)) = (argument(&args, "--url"), argument(&args, "--login")) else {
        eprintln!("usage: qadren-ispmanager-probe --url <https panel URL> --login <login>");
        eprintln!("The password is read from standard input.");
        return ExitCode::from(2);
    };
    let url = match PanelUrl::parse(&url) {
        Ok(url) => url,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(2);
        }
    };
    let password = match read_password() {
        Ok(password) if !password.is_empty() => password,
        _ => {
            eprintln!("no password on standard input");
            return ExitCode::from(2);
        }
    };
    let host = url.host().to_owned();
    let mut session = match Session::new(url, login.clone(), password.clone()) {
        Ok(session) => session,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(err) = session.authenticate() {
        println!("auth\t-\terror\t{err}");
        return ExitCode::FAILURE;
    }
    println!("auth\t-\tok\t");
    if let Some(database) = argument(&args, "--db-dump") {
        let (Some(port), Some(key)) = (
            argument(&args, "--ssh-port").and_then(|port| port.parse::<u16>().ok()),
            argument(&args, "--host-key"),
        ) else {
            eprintln!("--db-dump needs --ssh-port <port> and --host-key <SHA256 fingerprint>");
            return ExitCode::from(2);
        };
        let Ok(sftp) = SftpConnection::open(&host, port, &login, &password, &key) else {
            println!("sftp\t-\terror");
            return ExitCode::FAILURE;
        };
        return check_db_dump(&mut session, &sftp, &database);
    }
    for category in Category::ALL {
        let functions = category.list_functions();
        if functions.is_empty() {
            println!("{}\t-\tno_candidate\t", category.key());
        }
        for func in functions {
            let result = session.list(func);
            println!("{}\t{func}\t{}", category.key(), describe(&result));
        }
    }
    ExitCode::SUCCESS
}
