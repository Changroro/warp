use lazy_static::lazy_static;
use regex::Regex;

use crate::terminal::ssh::util::split_ssh_destination;

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

#[cfg(test)]
#[path = "login_prompt_tests.rs"]
mod tests;
