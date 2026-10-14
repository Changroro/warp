//! Lets ssh ask Warp for a saved SSH profile password instead of Warp typing it into the terminal.
//!
//! SSH manager connections run `ssh` with this binary as its `SSH_ASKPASS` helper. The helper
//! answers the first password prompt for the profile's own `user@host` from secure storage, and
//! relays every other prompt (host keys, jump hosts, one-time codes, retries) to the terminal.

use super::profile::{SshHostProfile, shell_quote_arg};

const PROFILE_ENV: &str = "WARP_SSH_ASKPASS_PROFILE";
/// `user@host`, or `@host` when the profile leaves the user to ssh.
const PRINCIPAL_ENV: &str = "WARP_SSH_ASKPASS_PRINCIPAL";

/// The environment assignments that make the `ssh` command after them ask this binary for
/// `profile`'s password, or `None` where the helper can't run.
pub fn env_prefix(profile: &SshHostProfile) -> Option<String> {
    if !cfg!(unix) {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    Some(format!(
        "SSH_ASKPASS={} SSH_ASKPASS_REQUIRE=force {PROFILE_ENV}={} {PRINCIPAL_ENV}={} ",
        shell_quote_arg(&exe.to_string_lossy()),
        profile.id,
        shell_quote_arg(&expected_principal(profile)),
    ))
}

/// Removes an [`env_prefix`] from `command`, so Warp still recognizes the `ssh` command after it.
pub fn strip_env_prefix(command: &str) -> &str {
    if !command.starts_with("SSH_ASKPASS=") {
        return command;
    }
    let Some(start) = command.find(&format!(" {PRINCIPAL_ENV}=")) else {
        return command;
    };
    command[start + 1..]
        .split_once(' ')
        .map_or("", |(_, rest)| rest.trim_start())
}

fn expected_principal(profile: &SshHostProfile) -> String {
    let user = profile.user.as_deref().unwrap_or_default();
    format!("{user}@{}", profile.host).to_lowercase()
}

/// The `user@host` that ssh names in a password prompt, in either the password or the
/// keyboard-interactive form. ssh writes the principal itself; only the trailing prompt text of a
/// keyboard-interactive prompt comes from the server.
fn prompt_principal(prompt: &str) -> Option<String> {
    let prompt = prompt.trim_end().to_lowercase();
    if let Some(principal) = prompt.strip_suffix("'s password:") {
        return Some(principal.to_string());
    }
    let (principal, text) = prompt.strip_prefix('(')?.split_once(") ")?;
    (text == "password:").then(|| principal.to_string())
}

/// Whether `prompt` asks for the account password of `expected`, an [`expected_principal`].
fn is_password_prompt_for(prompt: &str, expected: &str) -> bool {
    let Some(principal) = prompt_principal(prompt) else {
        return false;
    };
    if principal.contains(char::is_whitespace) {
        return false;
    }
    match expected.strip_prefix('@') {
        Some(host) => principal
            .rsplit_once('@')
            .is_some_and(|(user, prompt_host)| !user.is_empty() && prompt_host == host),
        None => principal == expected,
    }
}

/// Answers ssh's prompt and returns the exit code when ssh runs this binary as its askpass helper.
#[cfg(unix)]
pub fn run_if_requested() -> Option<i32> {
    let profile_id = std::env::var(PROFILE_ENV).ok()?;
    let mut args = std::env::args_os().skip(1);
    let prompt = args.next()?.to_string_lossy().into_owned();
    if args.next().is_some() {
        return None;
    }
    Some(match helper::answer(&profile_id, &prompt) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("Warp SSH password helper: {err:#}");
            1
        }
    })
}

#[cfg(unix)]
mod helper {
    use std::fs::{File, OpenOptions};
    use std::io::{BufRead as _, BufReader, Write as _};
    use std::os::fd::AsRawFd as _;
    use std::time::{Duration, SystemTime};

    use anyhow::{Context as _, bail};
    use nix::sys::termios::{self, LocalFlags, SetArg, Termios};
    use uuid::Uuid;
    use warp_core::channel::ChannelState;
    use warpui_extras::secure_storage::{self, Error as StorageError};
    use zeroize::Zeroizing;

    use super::{PRINCIPAL_ENV, is_password_prompt_for};
    use crate::ssh_manager::control_master::private_dir;
    use crate::ssh_manager::profile::ssh_profile_password_key;

    const STALE_MARKER_AGE: Duration = Duration::from_secs(24 * 60 * 60);

    pub(super) fn answer(profile_id: &str, prompt: &str) -> anyhow::Result<i32> {
        let hint = std::env::var("SSH_ASKPASS_PROMPT").unwrap_or_default();
        if hint == "none" {
            tty()?.write_all(format!("{prompt}\n").as_bytes())?;
            return Ok(0);
        }
        let expected = std::env::var(PRINCIPAL_ENV).unwrap_or_default();
        if hint != "confirm" && is_password_prompt_for(prompt, &expected) {
            // Any failure falls back to asking on the terminal, which is what ssh would do.
            let saved = first_password_prompt()
                .and_then(|first| {
                    if first {
                        stored_password(profile_id)
                    } else {
                        Ok(None)
                    }
                })
                .unwrap_or_else(|err| {
                    eprintln!("Warp couldn't enter the saved SSH password: {err:#}");
                    None
                });
            if let Some(password) = saved {
                std::io::stdout().write_all(format!("{}\n", password.as_str()).as_bytes())?;
                return Ok(0);
            }
        }

        let echo = hint == "confirm" || prompt.contains("(yes/no");
        let reply = relay(prompt, echo)?;
        if hint == "confirm" {
            return Ok(if reply.trim().eq_ignore_ascii_case("yes") {
                0
            } else {
                1
            });
        }
        std::io::stdout().write_all(format!("{}\n", reply.as_str()).as_bytes())?;
        Ok(0)
    }

    fn stored_password(profile_id: &str) -> anyhow::Result<Option<Zeroizing<String>>> {
        let profile_id = Uuid::parse_str(profile_id).context("invalid SSH profile id")?;
        let storage =
            secure_storage::platform(&ChannelState::data_domain(), warp_core::paths::state_dir());
        let password = match storage.read_value(&ssh_profile_password_key(profile_id)) {
            Ok(password) => Zeroizing::new(password),
            Err(StorageError::NotFound) => return Ok(None),
            Err(err) => return Err(err).context("failed to read the saved SSH password"),
        };
        if password.is_empty() || password.chars().any(char::is_control) {
            return Ok(None);
        }
        Ok(Some(password))
    }

    /// Claims the first password prompt of the calling ssh process, so a wrong saved password is
    /// tried once and the user types the retries.
    fn first_password_prompt() -> anyhow::Result<bool> {
        let dir = private_dir()?;
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let is_stale = entry
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                    .is_some_and(|age| age > STALE_MARKER_AGE);
                if is_stale && entry.file_name().to_string_lossy().starts_with("askpass-") {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        let marker = dir.join(format!("askpass-{}", std::os::unix::process::parent_id()));
        match OpenOptions::new().write(true).create_new(true).open(marker) {
            Ok(_) => Ok(true),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(err) => Err(err).context("failed to record the SSH password prompt"),
        }
    }

    fn tty() -> anyhow::Result<File> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .context("no terminal to ask on")
    }

    /// Asks the prompt on the terminal ssh runs in, the way ssh itself would without a helper.
    fn relay(prompt: &str, echo: bool) -> anyhow::Result<Zeroizing<String>> {
        let mut tty = tty()?;
        tty.write_all(prompt.as_bytes())?;
        let echo_off = if echo {
            None
        } else {
            Some(EchoOff::new(&tty)?)
        };
        let mut reply = Zeroizing::new(String::new());
        if BufReader::new(&tty).read_line(&mut reply)? == 0 {
            bail!("the terminal closed");
        }
        if echo_off.is_some() {
            drop(echo_off);
            tty.write_all(b"\n")?;
        }
        let trimmed = reply.trim_end_matches(['\r', '\n']).len();
        reply.truncate(trimmed);
        Ok(reply)
    }

    struct EchoOff<'a> {
        tty: &'a File,
        original: Termios,
    }

    impl<'a> EchoOff<'a> {
        fn new(tty: &'a File) -> anyhow::Result<Self> {
            let original = termios::tcgetattr(tty.as_raw_fd())?;
            let mut silent = original.clone();
            silent.local_flags.remove(LocalFlags::ECHO);
            termios::tcsetattr(tty.as_raw_fd(), SetArg::TCSANOW, &silent)?;
            Ok(Self { tty, original })
        }
    }

    impl Drop for EchoOff<'_> {
        fn drop(&mut self) {
            let _ = termios::tcsetattr(self.tty.as_raw_fd(), SetArg::TCSANOW, &self.original);
        }
    }
}

#[cfg(test)]
#[path = "askpass_tests.rs"]
mod tests;
