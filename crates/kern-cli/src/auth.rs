//! `kern login` / `kern logout` - store or remove registry credentials used by the OCI pull path
//! for private images. Credentials live in the owner-only (`0600`) file managed by
//! [`kern_common::registry_auth`]; the password is read from the terminal with echo off (or from
//! stdin when piped, so it stays out of shell history and `argv`).

use crate::error::Error;

/// Docker Hub's token service host - the default registry when none is given, matching the OCI
/// pull path's `DEFAULT_REGISTRY`.
const DEFAULT_REGISTRY: &str = "registry-1.docker.io";

/// `kern login [registry] [--username U] [--password-stdin | --password P]` - take (or prompt for) a
/// username, read a password without echo, CHECK the pair against the registry, and store it for
/// `registry` (default: Docker Hub).
pub fn login(
    registry: Option<&str>,
    username: Option<&str>,
    password: Option<&str>,
    password_stdin: bool,
) -> Result<(), Error> {
    let registry = registry.unwrap_or(DEFAULT_REGISTRY);
    // TWO SOURCES FOR ONE VALUE IS A QUESTION WITH NO ANSWER, so it is refused rather than resolved
    // by precedence: whichever kern picked, half the readers would expect the other.
    if password_stdin && password.is_some() {
        return Err(Error::Usage(
            "login: --password and --password-stdin both give the password - pass one",
        ));
    }
    let user =
        match username {
            Some(u) => u.to_string(),
            // `--password-stdin` WITHOUT `--username` HAS NO SAFE READING: stdin holds the password, so
            // prompting for the username would consume the password's line as the name. Docker refuses
            // this pair for the same reason, and so does this.
            None if password_stdin => return Err(Error::Usage(
                "login --password-stdin needs --username: stdin carries the password, so there is \
                 nothing left to read a username from",
            )),
            None => prompt(&format!("Username for {registry}: "))?,
        };
    if user.is_empty() {
        return Err(Error::Usage("login: username must not be empty"));
    }
    // `--password-stdin` READS STDIN WITH NO PROMPT. The pipeline it exists for is
    // `aws ecr get-login-password | kern login -u AWS --password-stdin <registry>`, and a prompt on
    // stderr there is noise in a CI log rather than a question anyone answers.
    let pass = match (password, password_stdin) {
        (Some(p), _) => {
            eprintln!(
                "kern: warning: --password puts the password in this process's argv, where any \
                 process running as you can read it from /proc. Prefer --password-stdin"
            );
            p.to_string()
        }
        (None, true) => read_line_raw()?,
        (None, false) => read_password(&format!("Password for {user}@{registry}: "))?,
    };
    if pass.is_empty() {
        return Err(Error::Usage("login: password must not be empty"));
    }
    // VERIFIED BEFORE IT IS STORED, and the order is the whole point. The store holds one entry per
    // registry, so writing first and checking later would replace a working credential with a
    // rejected one; and a `login` that cannot fail turns an expired CI token into a `push` error
    // several steps downstream.
    kern_oci::verify_credentials(registry, &user, &pass).map_err(|e| Error::Oci(e.to_string()))?;
    kern_common::registry_auth::store(registry, &user, &pass)
        .map_err(|e| Error::Sandbox(format!("storing credentials: {e}")))?;
    println!("logged in to {registry} as {user}");
    Ok(())
}

/// One line from stdin, with no prompt and no terminal handling: what `--password-stdin` reads.
/// A trailing newline is stripped (a pipeline's `echo` adds one) and nothing else is trimmed, so a
/// password with leading or trailing spaces survives.
fn read_line_raw() -> Result<String, Error> {
    use std::io::BufRead;
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| Error::Sandbox(format!("reading password from stdin: {e}")))?;
    Ok(line.trim_end_matches(['\n', '\r']).to_string())
}

/// `kern logout [registry]` - remove the stored credentials for `registry` (default: Docker Hub).
pub fn logout(registry: Option<&str>) -> Result<(), Error> {
    let registry = registry.unwrap_or(DEFAULT_REGISTRY);
    let removed = kern_common::registry_auth::remove(registry)
        .map_err(|e| Error::Sandbox(format!("removing credentials: {e}")))?;
    if removed {
        println!("logged out of {registry}");
    } else {
        println!("not logged in to {registry}");
    }
    Ok(())
}

/// Print `prompt` to stderr (so it doesn't pollute piped stdout) and read a line from stdin.
fn prompt(prompt: &str) -> Result<String, Error> {
    use std::io::{BufRead, Write};
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| Error::Sandbox(format!("reading input: {e}")))?;
    Ok(line.trim_end_matches(['\n', '\r']).to_string())
}

/// Read a password without echoing it. Turns off terminal `ECHO` for the read when stdin is a tty;
/// when stdin is piped (not a tty) it just reads a line, so CI/scripts can `printf pass | kern login`.
fn read_password(prompt_text: &str) -> Result<String, Error> {
    use std::io::{BufRead, Write};
    let is_tty = unsafe { libc::isatty(0) } == 1;
    eprint!("{prompt_text}");
    let _ = std::io::stderr().flush();

    // Save termios, clear ECHO, read, restore - best-effort and panic-safe (restored on every path).
    let mut saved: libc::termios = unsafe { std::mem::zeroed() };
    let echo_off = is_tty && unsafe { libc::tcgetattr(0, &mut saved) } == 0;
    if echo_off {
        let mut raw = saved;
        raw.c_lflag &= !libc::ECHO;
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &raw) };
    }
    let mut line = String::new();
    let read = std::io::stdin().lock().read_line(&mut line);
    if echo_off {
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &saved) };
        eprintln!(); // the user's Enter wasn't echoed - move to a fresh line
    }
    read.map_err(|e| Error::Sandbox(format!("reading password: {e}")))?;
    Ok(line.trim_end_matches(['\n', '\r']).to_string())
}
