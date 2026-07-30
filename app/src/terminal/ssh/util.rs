use std::path::Path;

use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const SSH_COMMAND_LAUNCHER: [&str; 2] = ["command", "ssh"];

fn new_profile_id() -> Uuid {
    Uuid::new_v4()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(description = "A saved SSH jump host configuration.")]
pub struct SshJumpHost {
    #[serde(default)]
    pub source_profile_id: Option<Uuid>,
    pub host: String,
    pub user: Option<String>,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    pub identity_file: Option<String>,
}

impl settings_value::SettingsValue for SshJumpHost {}

impl SshJumpHost {
    pub fn target(&self) -> String {
        ssh_target(&self.user, &self.host, self.port, true)
    }

    fn destination(&self) -> String {
        ssh_target(&self.user, &self.host, self.port, false)
    }

    fn host_port(&self) -> String {
        host_with_port(&self.host, self.port)
    }

    fn push_proxy_command_options(&self, argv: &mut Vec<String>) {
        if let Some(identity_file) = self
            .identity_file
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            argv.push("-i".to_string());
            argv.push(escape_openssh_percent(identity_file));
        }

        if self.port != default_ssh_port() {
            argv.push("-p".to_string());
            argv.push(self.port.to_string());
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(description = "A saved SSH host profile.")]
pub struct SshHostProfile {
    #[serde(default = "new_profile_id")]
    pub id: Uuid,
    pub name: String,
    pub host: String,
    pub user: Option<String>,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    pub identity_file: Option<String>,
    #[serde(default)]
    pub jump_hosts: Vec<SshJumpHost>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl settings_value::SettingsValue for SshHostProfile {}

impl SshHostProfile {
    pub fn new(name: String, host: String) -> Self {
        Self {
            id: new_profile_id(),
            name,
            host,
            user: None,
            port: default_ssh_port(),
            identity_file: None,
            jump_hosts: Vec::new(),
            tags: Vec::new(),
        }
    }

    pub fn target(&self) -> String {
        ssh_target(&self.user, &self.host, self.port, false)
    }

    pub fn to_ssh_command(&self) -> String {
        let mut argv = SSH_COMMAND_LAUNCHER
            .iter()
            .map(|arg| (*arg).to_string())
            .collect::<Vec<_>>();

        if let Some(identity_file) = self
            .identity_file
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            argv.push("-i".to_string());
            argv.push(identity_file.to_string());
        }

        if self.port != default_ssh_port() {
            argv.push("-p".to_string());
            argv.push(self.port.to_string());
        }

        if !self.jump_hosts.is_empty() {
            if self.jump_hosts.iter().any(|jump_host| {
                jump_host
                    .identity_file
                    .as_deref()
                    .is_some_and(|value| !value.is_empty())
            }) {
                argv.push("-o".to_string());
                argv.push(format!(
                    "ProxyCommand={}",
                    proxy_command_for_jump_hosts(
                        &self.jump_hosts,
                        &host_with_port(&self.host, self.port)
                    )
                ));
            } else {
                argv.push("-J".to_string());
                argv.push(
                    self.jump_hosts
                        .iter()
                        .map(SshJumpHost::target)
                        .collect::<Vec<_>>()
                        .join(","),
                );
            }
        }

        argv.push("--".to_string());
        argv.push(self.target());
        argv.into_iter()
            .map(|arg| shell_quote_arg(&arg))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn proxy_command_for_jump_hosts(jump_hosts: &[SshJumpHost], destination: &str) -> String {
    let mut proxy_command: Option<String> = None;
    for (index, jump_host) in jump_hosts.iter().enumerate() {
        let next_destination = jump_hosts
            .get(index + 1)
            .map(SshJumpHost::host_port)
            .unwrap_or_else(|| destination.to_string());

        let mut argv = vec!["ssh".to_string()];
        if let Some(previous_proxy_command) = proxy_command.take() {
            argv.push("-o".to_string());
            argv.push(format!(
                "ProxyCommand={}",
                escape_openssh_percent(&previous_proxy_command)
            ));
        }
        jump_host.push_proxy_command_options(&mut argv);
        argv.push("-W".to_string());
        argv.push(escape_openssh_percent(&next_destination));
        argv.push("--".to_string());
        argv.push(escape_openssh_percent(&jump_host.destination()));

        proxy_command = Some(
            argv.into_iter()
                .map(|arg| shell_quote_arg(&arg))
                .collect::<Vec<_>>()
                .join(" "),
        );
    }

    proxy_command.unwrap_or_default()
}

pub fn default_ssh_port() -> u16 {
    22
}

/// A profile id replaced by [`normalize_ssh_host_profiles`]; secrets stored under `previous_id` no
/// longer resolve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SshProfileIdReassignment {
    pub previous_id: Uuid,
    pub new_id: Uuid,
}

/// Replaces nil and duplicate profile ids with fresh ones and normalizes zero ports, returning
/// every id reassignment so callers can migrate or discard secrets keyed by the previous id.
#[must_use]
pub fn normalize_ssh_host_profiles(
    profiles: &mut [SshHostProfile],
) -> Vec<SshProfileIdReassignment> {
    let mut seen = std::collections::HashSet::new();
    let mut reassignments = Vec::new();
    for profile in profiles {
        if profile.id.is_nil() || !seen.insert(profile.id) {
            let previous_id = profile.id;
            profile.id = Uuid::new_v4();
            seen.insert(profile.id);
            reassignments.push(SshProfileIdReassignment {
                previous_id,
                new_id: profile.id,
            });
        }
        if profile.port == 0 {
            profile.port = default_ssh_port();
        }
        for jump_host in &mut profile.jump_hosts {
            if jump_host.port == 0 {
                jump_host.port = default_ssh_port();
            }
        }
    }
    reassignments
}

pub fn ssh_profile_password_key(id: Uuid) -> String {
    format!("ssh-profile-password:{id}")
}

pub fn shell_quote_arg(arg: &str) -> String {
    shell_words::quote(arg).into_owned()
}

/// Renders a `host[:port]` destination in the bracketed form OpenSSH requires for IPv6 literals.
fn host_with_port(host: &str, port: u16) -> String {
    if is_ipv6_literal(host) {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn is_ipv6_literal(host: &str) -> bool {
    host.contains(':') && !host.starts_with('[')
}

/// Escapes one level of OpenSSH `%` token expansion so the value reaches the next `ssh` literally.
fn escape_openssh_percent(value: &str) -> String {
    value.replace('%', "%%")
}

fn ssh_target(user: &Option<String>, host: &str, port: u16, include_port: bool) -> String {
    let mut target = String::new();
    if let Some(user) = user.as_deref().filter(|value| !value.is_empty()) {
        target.push_str(user);
        target.push('@');
    }
    if include_port && is_ipv6_literal(host) {
        target.push('[');
        target.push_str(host);
        target.push(']');
    } else {
        target.push_str(host);
    }
    if include_port && port != default_ssh_port() {
        target.push(':');
        target.push_str(&port.to_string());
    }
    target
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshPromptPrincipal {
    pub user: Option<String>,
    pub host: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SshLoginPromptKind {
    HostPassword {
        principal: Option<SshPromptPrincipal>,
    },
    KeyPassphrase {
        identity_file: String,
    },
}

pub fn parse_ssh_login_prompt(output: &str) -> Option<SshLoginPromptKind> {
    lazy_static! {
        static ref PASSWORD_PROMPT: Regex = Regex::new(r"^(?P<principal>.*)'s password:\s*$")
            .expect("password prompt regex invalid");
        static ref PAM_PASSWORD_PROMPT: Regex =
            Regex::new(r"^(?:\((?P<principal>[^()]*)\)\s*)?Password:\s*$")
                .expect("PAM password prompt regex invalid");
        static ref PASSPHRASE_PROMPT: Regex =
            Regex::new(r#"(?i)^enter passphrase for key ['"](?P<identity_file>.*)['"]:\s*$"#)
                .expect("passphrase prompt regex invalid");
    }

    let last_line = output
        .trim_end_matches(['\r', '\n'])
        .split(['\r', '\n'])
        .rev()
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .trim_end_matches('\r');

    if let Some(captures) = PASSWORD_PROMPT.captures(last_line) {
        let principal = captures
            .name("principal")
            .and_then(|principal| principal.as_str().split_whitespace().next_back())
            .and_then(parse_ssh_prompt_principal);
        return Some(SshLoginPromptKind::HostPassword { principal });
    }

    if let Some(captures) = PAM_PASSWORD_PROMPT.captures(last_line) {
        let principal = captures
            .name("principal")
            .map(|principal| principal.as_str().trim())
            .and_then(parse_pam_prompt_principal);
        return Some(SshLoginPromptKind::HostPassword { principal });
    }

    PASSPHRASE_PROMPT
        .captures(last_line)
        .map(|captures| SshLoginPromptKind::KeyPassphrase {
            identity_file: captures["identity_file"].to_string(),
        })
}

/// OpenSSH writes `user@host` inside a PAM prompt's parentheses; without `@` no host is named.
fn parse_pam_prompt_principal(value: &str) -> Option<SshPromptPrincipal> {
    let (user, host) = value.rsplit_once('@')?;
    (!user.is_empty() && !host.is_empty()).then(|| SshPromptPrincipal {
        user: Some(user.to_string()),
        host: host.to_string(),
    })
}

fn parse_ssh_prompt_principal(value: &str) -> Option<SshPromptPrincipal> {
    let (user, host) = split_ssh_destination(value);
    (!host.is_empty()).then(|| SshPromptPrincipal {
        user: user.map(ToString::to_string),
        host: host.to_string(),
    })
}

pub fn split_ssh_destination(destination: &str) -> (Option<&str>, &str) {
    match destination.rsplit_once('@') {
        Some((user, host)) if !user.is_empty() => (Some(user), host),
        Some((_, host)) => (None, host),
        None => (None, destination),
    }
}

/// Converts a multiline bash or zsh script to one line by turning newlines into semicolons or
/// deleting them, as appropriate.
///
/// Extra semicolons are a syntax error in bash, so this is careful to avoid adding them except
/// where necessary.
///
/// This function exists because there's a strange macOS ssh server bug where sending a lot of data
/// containing newlines to a shell results in data corruption.
pub fn convert_script_to_one_line(script: &str) -> String {
    lazy_static! {
        static ref EXTRA_SPACES_REGEX: Regex = Regex::new(r"\n+\s*").expect("invalid regex");
        static ref NO_SEMICOLON_REGEX: Regex =
            Regex::new("(; ?|\\{|do|then|else|in)\n").expect("invalid regex");
        static ref REMOVE_COMMENTS_REGEX: Regex = Regex::new(r"(?m)^ *#.*").expect("invalid regex");
        static ref REMOVE_LEADING_NEWLINES: Regex = Regex::new(r"^\n*").expect("invalid regex");
    };
    let script = REMOVE_COMMENTS_REGEX.replace_all(script, "");
    let script = REMOVE_LEADING_NEWLINES.replace_all(&script, "");
    let script = EXTRA_SPACES_REGEX.replace_all(&script, "\n");
    let script = NO_SEMICOLON_REGEX.replace_all(&script, "$1 ");
    let mut script = script.replace('\n', ";");
    script.push('\n');
    script
}

pub enum SshLoginState {
    LastLogin,
    NonSshOutput,
    Authenticating,
    PromptDetected,
}

/// Reads the contents of the output grid to determine SSH login state. Returns [SshLoginState::LastLogin] if
/// "Last login:" is detected in the output. Returns [SshLoginState::NonSshOutput] if certain keywords
/// known to be a part of ssh login prompts are found in the current last line of command output. The
/// "password" and "Password" are for password authentication. "passphrase" is intended to cover authentication
/// by public key. And "yes/no" relates to trust-on-first-use prompts for host-based authentication.
pub fn check_ssh_login_state(block_output: &str) -> SshLoginState {
    lazy_static! {
        // Common final prompt characters followed by a space.
        static ref PROMPT_REGEX: Regex = Regex::new(r"[$#%>❯│⟫»▶λ→] $").expect("invalid regex");
    };

    let mut last_line = None;

    for line in block_output.lines() {
        if line.starts_with("Last login:") {
            return SshLoginState::LastLogin;
        }
        // With an iterator, there's no way to know if it's the last element so
        // we overwrite last_line at each iteration.
        last_line = Some(line);
    }

    last_line.map_or(SshLoginState::Authenticating, |line| {
        if line.contains("password")
            || line.contains("Password")
            || line.contains("passphrase")
            || line.contains("yes/no")
            || line.contains("Please type")
            || line.contains("'yes'")
            || line.contains("Confirm user presence")
            || line.starts_with("Enter ")
            || line.starts_with("Allow ")
        {
            SshLoginState::Authenticating
        } else if PROMPT_REGEX.is_match(line) {
            SshLoginState::PromptDetected
        } else {
            SshLoginState::NonSshOutput
        }
    })
}

/// Represents the parsed components of an interactive SSH command.
/// For some [`SshWarpifyCommand`]s, we do not support parsing
/// a host or port In these cases, we can still parse to a valid
/// empty `InteractiveSshCommand` to indicate that we did
/// successfully detect an interactive SSH command.
#[derive(Clone, Debug, Default)]
pub struct InteractiveSshCommand {
    pub host: Option<String>,
    pub port: Option<String>,
    pub user: Option<String>,
}

impl InteractiveSshCommand {
    /// Parses ssh commands of the form `ssh ...`.
    /// Only returns an `InteractiveSshCommand` if we determine the command is interactive.
    fn parse_ssh_command(command: &str) -> Option<InteractiveSshCommand> {
        let command = if let Some(suffix) = command.strip_prefix("command ") {
            suffix
        } else {
            command
        };
        let tokens = parse_ssh_command_tokens(command)?;
        let mut host: Option<String> = None;
        let mut port: Option<String> = None;
        let mut login_name: Option<String> = None;

        let mut i = 1;
        while i < tokens.len() {
            match tokens[i].as_str() {
                // -T or -W imply a non-interactive session.
                "-T" | "-W" => return None,

                "-p" => {
                    i += 1;
                    if i < tokens.len() {
                        port = Some(tokens[i].clone());
                    } else {
                        return None;
                    }
                }

                "-l" => {
                    i += 1;
                    if i < tokens.len() {
                        login_name = Some(tokens[i].clone());
                    } else {
                        return None;
                    }
                }

                // SSH option that doesn't change interactivity and require an argument: Skip the next item.
                "-B" | "-b" | "-c" | "-D" | "-E" | "-e" | "-F" | "-I" | "-i" | "-J" | "-L"
                | "-m" | "-O" | "-o" | "-P" | "-Q" | "-R" | "-S" | "-w" => {
                    i += 1;
                }

                // SSH option(s) that don't change interactivity.
                arg if arg.starts_with('-') => {}

                // Otherwise, it's a positional argument (e.g., hostname, command to run)
                pos_arg => {
                    // If we detect multiple positional args, there's some type of unknown command formulation.
                    if host.is_some() {
                        return None;
                    }
                    host = Some(pos_arg.to_string());
                }
            }
            i += 1;
        }

        let user = host
            .as_deref()
            .and_then(|destination| split_ssh_destination(destination).0)
            .map(ToString::to_string)
            .or(login_name);
        Some(InteractiveSshCommand { host, port, user })
    }
}

pub enum SshLikeCommand {
    Gcloud,
    ElasticBeanstalk,
    DigitalOceanDroplet,
}

/// Represents the different kinds of commands we recognize as starting an interactive SSH
/// session. `Ssh` means a literal `ssh` command, where all other commands (e.g. `gcloud
/// compute ssh`) are categorized as SSH-like commands.
pub enum SshWarpifyCommand {
    Ssh,
    SshLike(SshLikeCommand),
}

impl SshWarpifyCommand {
    /// Not a literal `ssh` command, but another command that starts an interactive SSH
    /// session.
    pub fn is_ssh_like_command(&self) -> bool {
        matches!(self, SshWarpifyCommand::SshLike(_))
    }
}

lazy_static! {
    static ref INTERACTIVE_SSH: Regex = Regex::new(r"^ssh\s+").expect("interactive SSH regex invalid");

    /// Matches "gcloud compute ssh" for connecting to GCP VMs.
    static ref GCLOUD_REGEX: Regex = Regex::new(r"^gcloud\s+compute\s+ssh\s.+").expect("gcloud SSH regex invalid");

    /// Matches "eb ssh" for connecting to AWS Elastic Beanstalk VMs.
    static ref ELASTIC_BEANSTALK_REGEX: Regex = Regex::new(r"^eb\s+ssh\s.+").expect("elastic beanstalk SSH regex invalid");

    /// Matches "doctl compute ssh" for connecting to a digital ocean droplet.
    static ref DIGITAL_OCEAN_DROPLET_REGEX: Regex = Regex::new(r"^doctl\s+compute\s+ssh\s.+").expect("digital ocean SSH regex invalid");
}

impl SshWarpifyCommand {
    pub fn matches(command: &str) -> Option<SshWarpifyCommand> {
        let command = if let Some(suffix) = command.strip_prefix("command ") {
            suffix
        } else {
            command
        };
        if INTERACTIVE_SSH.is_match(command) {
            Some(SshWarpifyCommand::Ssh)
        } else if GCLOUD_REGEX.is_match(command) {
            Some(SshWarpifyCommand::SshLike(SshLikeCommand::Gcloud))
        } else if ELASTIC_BEANSTALK_REGEX.is_match(command) {
            Some(SshWarpifyCommand::SshLike(SshLikeCommand::ElasticBeanstalk))
        } else if DIGITAL_OCEAN_DROPLET_REGEX.is_match(command) {
            Some(SshWarpifyCommand::SshLike(
                SshLikeCommand::DigitalOceanDroplet,
            ))
        } else {
            None
        }
    }
}

pub fn parse_interactive_ssh_command(command: &str) -> Option<InteractiveSshCommand> {
    match SshWarpifyCommand::matches(command) {
        Some(SshWarpifyCommand::Ssh) => InteractiveSshCommand::parse_ssh_command(command),
        Some(SshWarpifyCommand::SshLike(SshLikeCommand::Gcloud)) => {
            Some(InteractiveSshCommand::default())
        }
        Some(SshWarpifyCommand::SshLike(SshLikeCommand::ElasticBeanstalk)) => {
            Some(InteractiveSshCommand::default())
        }
        Some(SshWarpifyCommand::SshLike(SshLikeCommand::DigitalOceanDroplet)) => {
            Some(InteractiveSshCommand::default())
        }
        None => None,
    }
}

fn parse_ssh_command_tokens(command: &str) -> Option<Vec<String>> {
    let Ok(tokens) = shell_words::split(command) else {
        return None;
    };

    // Cases: "", "ls", "ssh-add-key"
    if tokens.is_empty() || tokens[0] != "ssh" {
        return None;
    }
    Some(tokens)
}

/// Creates an sftp command that copies a given local file into the pwd in the warpified ssh session.
pub fn transfer_file_sftp_command(
    local_file_path: String,
    ssh_host: String,
    ssh_port: Option<String>,
    pwd: Option<String>,
) -> Option<String> {
    // "sftp "
    let mut command = String::from("sftp ");

    // "sftp -P 2222"
    if let Some(port) = ssh_port {
        command += &format!("-P {port} ");
    }

    // "sftp -P 2222 sshuser@127.0.0.1 <<< "put "
    command += &ssh_host;
    command += " <<< \"put ";

    // "sftp -P 2222 sshuser@127.0.0.1 <<< "put -r"
    let is_dir = Path::new(&local_file_path)
        .metadata()
        .is_ok_and(|m| m.is_dir());
    if is_dir {
        command += "-r "
    }

    // "sftp -P 2222 sshuser@127.0.0.1 <<< "put -r \"path/to/local/file\""
    command += &format!("\\\"{}\\\"", &local_file_path);

    // "sftp -P 2222 sshuser@127.0.0.1 <<< "put -r path/to/local/file pwd/on/remote"
    if let Some(pwd) = pwd {
        command += " ";
        command += &format!("\\\"{}\\\"", &pwd);
    }

    // "sftp -P 2222 sshuser@127.0.0.1 <<< "put -r path/to/local/file pwd/on/remote""
    command += "\"";

    Some(command)
}

#[cfg(test)]
#[path = "util_tests.rs"]
mod tests;
