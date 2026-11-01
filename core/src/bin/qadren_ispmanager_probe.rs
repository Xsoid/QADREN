//! Gate 0 read-only probe for an ISPmanager 6 panel (D-027, proposed).
//!
//! Usage: `<password source> | qadren-ispmanager-probe --url <panel URL> --login <login>`.
//! The password comes from standard input, never from an argument. The output
//! has result classes and field names only: no values, counts, or session ID.
use qadren_core::ispmanager::{Category, IspError, ListOutcome, PanelUrl, Session};
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
    let mut session = match Session::new(url, login, password) {
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
