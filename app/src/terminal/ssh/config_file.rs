//! `~/.ssh/config` as the source of truth for SSH host profiles.
//!
//! Profiles live in the user's real ssh config so they work from any terminal (`ssh <alias>`),
//! not just inside Warp. Fields that have no standard ssh_config directive (our profile id, tags,
//! per-jump-host identity files) are round-tripped through `# warp-*` comments, which every other
//! ssh client ignores.
//!
//! Writing is deliberately conservative: the file is rewritten line-by-line, and anything we do not
//! understand — global options, `Include`/`Match`, unknown directives such as `IdentitiesOnly`,
//! inline comments, blank lines, hand-written hosts — is preserved verbatim. Passwords are never
//! written here; they stay in secure storage keyed by profile id.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use uuid::Uuid;

use super::util::{default_ssh_port, SshHostProfile, SshJumpHost};

const WARP_ID_PREFIX: &str = "# warp-id:";
const WARP_TAGS_PREFIX: &str = "# warp-tags:";
const WARP_JUMP_IDENTITIES_PREFIX: &str = "# warp-jump-identities:";

/// Path to the user's ssh config (`~/.ssh/config`).
pub fn default_ssh_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".ssh").join("config"))
}

/// One `Host` stanza, keeping its original lines so unknown directives survive a rewrite.
#[derive(Clone, Debug)]
struct HostBlock {
    warp_id: Option<Uuid>,
    tags: Vec<String>,
    jump_identities: Vec<String>,
    alias: String,
    /// Body lines exactly as they appeared (including unknown directives and inline comments).
    body: Vec<String>,
}

/// The config file as an ordered list of things we rewrite (host blocks) and things we don't.
#[derive(Clone, Debug)]
enum Segment {
    /// Anything outside a Host stanza: global options, comments, blank lines, Include/Match.
    Raw(String),
    Host(HostBlock),
}

fn parse(contents: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut pending_id: Option<Uuid> = None;
    let mut pending_tags: Vec<String> = Vec::new();
    let mut pending_jump_identities: Vec<String> = Vec::new();
    let mut current: Option<HostBlock> = None;

    for line in contents.lines() {
        let trimmed = line.trim();

        // Our metadata comments attach to the Host stanza that follows them.
        if let Some(value) = strip_prefix_ci(trimmed, WARP_ID_PREFIX) {
            pending_id = Uuid::parse_str(value.trim()).ok();
            continue;
        }
        if let Some(value) = strip_prefix_ci(trimmed, WARP_TAGS_PREFIX) {
            pending_tags = split_csv(value);
            continue;
        }
        if let Some(value) = strip_prefix_ci(trimmed, WARP_JUMP_IDENTITIES_PREFIX) {
            pending_jump_identities = split_csv(value);
            continue;
        }

        if let Some(alias) = parse_host_header(trimmed) {
            if let Some(block) = current.take() {
                segments.push(Segment::Host(block));
            }
            current = Some(HostBlock {
                warp_id: pending_id.take(),
                tags: std::mem::take(&mut pending_tags),
                jump_identities: std::mem::take(&mut pending_jump_identities),
                alias,
                body: Vec::new(),
            });
            continue;
        }

        match current.as_mut() {
            Some(block) => block.body.push(line.to_string()),
            None => segments.push(Segment::Raw(line.to_string())),
        }
    }

    if let Some(block) = current.take() {
        segments.push(Segment::Host(block));
    }
    segments
}

fn serialize(segments: &[Segment]) -> String {
    let mut out = String::new();
    for segment in segments {
        match segment {
            Segment::Raw(line) => {
                out.push_str(line);
                out.push('\n');
            }
            Segment::Host(block) => {
                if let Some(id) = block.warp_id {
                    out.push_str(&format!("{WARP_ID_PREFIX} {id}\n"));
                }
                if !block.tags.is_empty() {
                    out.push_str(&format!("{WARP_TAGS_PREFIX} {}\n", block.tags.join(",")));
                }
                if !block.jump_identities.is_empty() {
                    out.push_str(&format!(
                        "{WARP_JUMP_IDENTITIES_PREFIX} {}\n",
                        block.jump_identities.join(",")
                    ));
                }
                out.push_str(&format!("Host {}\n", block.alias));
                for line in &block.body {
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
    }
    out
}

impl HostBlock {
    fn directive(&self, name: &str) -> Option<String> {
        self.body.iter().find_map(|line| {
            let (key, value) = split_directive(line)?;
            key.eq_ignore_ascii_case(name).then_some(value)
        })
    }

    /// Replace a directive's value in place, keeping its position; append if absent; drop if `None`.
    fn set_directive(&mut self, name: &str, value: Option<impl AsRef<str>>) {
        let indent = self.indent();
        let value = value.as_ref().map(AsRef::as_ref);
        let existing = self.body.iter().position(|line| {
            split_directive(line)
                .map(|(key, _)| key.eq_ignore_ascii_case(name))
                .unwrap_or(false)
        });

        match (existing, value) {
            (Some(idx), Some(value)) => {
                self.body[idx] = format!("{indent}{name} {value}");
            }
            (Some(idx), None) => {
                self.body.remove(idx);
            }
            (None, Some(value)) => {
                let insert_at = self
                    .body
                    .iter()
                    .rposition(|line| !line.trim().is_empty())
                    .map(|idx| idx + 1)
                    .unwrap_or(0);
                self.body.insert(insert_at, format!("{indent}{name} {value}"));
            }
            (None, None) => {}
        }
    }

    /// Match the file's existing indentation so rewrites look hand-written.
    fn indent(&self) -> String {
        self.body
            .iter()
            .find(|line| !line.trim().is_empty() && line.starts_with(char::is_whitespace))
            .map(|line| {
                line.chars()
                    .take_while(|c| c.is_whitespace())
                    .collect::<String>()
            })
            .unwrap_or_else(|| "    ".to_string())
    }

    fn to_profile(&self) -> SshHostProfile {
        let host = self
            .directive("HostName")
            .unwrap_or_else(|| self.alias.clone());
        let port = self
            .directive("Port")
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(default_ssh_port);
        let jump_hosts = self
            .directive("ProxyJump")
            .map(|value| parse_proxy_jump(&value, &self.jump_identities))
            .unwrap_or_default();

        SshHostProfile {
            // Hand-written hosts have no warp id yet; give them an ephemeral one. It is persisted
            // (as a `# warp-id:` comment) the first time the user saves the profile from Warp,
            // which is also when a password could be attached to it.
            id: self.warp_id.unwrap_or_else(Uuid::new_v4),
            name: self.alias.clone(),
            host,
            user: self.directive("User"),
            port,
            identity_file: self.directive("IdentityFile"),
            jump_hosts,
            tags: self.tags.clone(),
        }
    }

    fn apply_profile(&mut self, profile: &SshHostProfile) {
        self.warp_id = Some(profile.id);
        self.tags = profile.tags.clone();
        self.alias = profile.name.clone();

        self.set_directive("HostName", Some(&profile.host));
        self.set_directive("User", non_empty(profile.user.as_deref()));
        self.set_directive(
            "Port",
            (profile.port != default_ssh_port()).then(|| profile.port.to_string()),
        );
        self.set_directive("IdentityFile", non_empty(profile.identity_file.as_deref()));

        if profile.jump_hosts.is_empty() {
            self.set_directive("ProxyJump", None::<&str>);
            self.jump_identities.clear();
        } else {
            self.set_directive("ProxyJump", Some(&format_proxy_jump(&profile.jump_hosts)));
            self.jump_identities = profile
                .jump_hosts
                .iter()
                .map(|jump| jump.identity_file.clone().unwrap_or_default())
                .collect();
            if self.jump_identities.iter().all(String::is_empty) {
                self.jump_identities.clear();
            }
        }
    }

    fn new_for(profile: &SshHostProfile) -> Self {
        let mut block = HostBlock {
            warp_id: None,
            tags: Vec::new(),
            jump_identities: Vec::new(),
            alias: profile.name.clone(),
            body: Vec::new(),
        };
        block.apply_profile(profile);
        block
    }
}

/// Read every `Host` stanza in `~/.ssh/config` as a profile. Hand-written hosts are included.
pub fn load_profiles() -> Result<Vec<SshHostProfile>> {
    let Some(path) = default_ssh_config_path() else {
        return Ok(Vec::new());
    };
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;

    Ok(parse(&contents)
        .iter()
        .filter_map(|segment| match segment {
            Segment::Host(block) => Some(block.to_profile()),
            Segment::Raw(_) => None,
        })
        .collect())
}

/// Create or update the stanza for `profile`, leaving everything else in the file untouched.
pub fn upsert_profile(profile: &SshHostProfile) -> Result<()> {
    edit(|segments| {
        let existing = segments.iter_mut().find_map(|segment| match segment {
            Segment::Host(block)
                if block.warp_id == Some(profile.id) || block.alias == profile.name =>
            {
                Some(block)
            }
            _ => None,
        });

        match existing {
            Some(block) => block.apply_profile(profile),
            None => {
                if !matches!(segments.last(), Some(Segment::Raw(line)) if line.trim().is_empty()) {
                    segments.push(Segment::Raw(String::new()));
                }
                segments.push(Segment::Host(HostBlock::new_for(profile)));
            }
        }
    })
}

/// Remove the stanza owned by `id`. Hand-written hosts (no warp id) are never removed.
pub fn remove_profile(id: Uuid) -> Result<()> {
    edit(|segments| {
        segments.retain(|segment| !matches!(segment, Segment::Host(block) if block.warp_id == Some(id)));
    })
}

/// Rewrite the config atomically, keeping a `.warp.bak` copy of the previous contents.
fn edit(mutate: impl FnOnce(&mut Vec<Segment>)) -> Result<()> {
    let Some(path) = default_ssh_config_path() else {
        anyhow::bail!("Could not determine the location of ~/.ssh/config");
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    let contents = if path.exists() {
        fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?
    } else {
        String::new()
    };

    let mut segments = parse(&contents);
    mutate(&mut segments);
    let updated = serialize(&segments);

    if path.exists() {
        let backup = path.with_extension("warp.bak");
        fs::copy(&path, &backup)
            .with_context(|| format!("Failed to back up {}", path.display()))?;
    }

    // Write to a temp file in the same directory, then rename, so a crash can't truncate the config.
    let temp = path.with_extension("warp.tmp");
    let mut file = fs::File::create(&temp)
        .with_context(|| format!("Failed to create {}", temp.display()))?;
    file.write_all(updated.as_bytes())
        .with_context(|| format!("Failed to write {}", temp.display()))?;
    file.sync_all().ok();
    drop(file);

    set_private_permissions(&temp)?;
    fs::rename(&temp, &path)
        .with_context(|| format!("Failed to replace {}", path.display()))?;
    Ok(())
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("Failed to set permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

fn parse_host_header(trimmed: &str) -> Option<String> {
    let (key, value) = split_directive(trimmed)?;
    key.eq_ignore_ascii_case("Host").then_some(value)
}

/// Split `Key value  # trailing comment` into `(Key, value)`, dropping the inline comment.
fn split_directive(line: &str) -> Option<(&str, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let (key, rest) = trimmed.split_once(char::is_whitespace)?;
    let value = rest.split('#').next().unwrap_or("").trim();
    (!value.is_empty()).then(|| (key, value.to_string()))
}

fn strip_prefix_ci<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    line.get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(prefix))
        .map(|_| &line[prefix.len()..])
}

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// `user@host:port,user2@host2` — the standard ProxyJump chain format.
fn format_proxy_jump(jump_hosts: &[SshJumpHost]) -> String {
    jump_hosts
        .iter()
        .map(|jump| {
            let mut target = String::new();
            if let Some(user) = non_empty(jump.user.as_deref()) {
                target.push_str(&user);
                target.push('@');
            }
            target.push_str(&jump.host);
            if jump.port != default_ssh_port() {
                target.push(':');
                target.push_str(&jump.port.to_string());
            }
            target
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_proxy_jump(value: &str, identities: &[String]) -> Vec<SshJumpHost> {
    value
        .split(',')
        .map(str::trim)
        .filter(|hop| !hop.is_empty())
        .enumerate()
        .map(|(idx, hop)| {
            let (user, rest) = match hop.split_once('@') {
                Some((user, rest)) => (non_empty(Some(user)), rest),
                None => (None, hop),
            };
            let (host, port) = match rest.rsplit_once(':') {
                Some((host, port)) => match port.parse() {
                    Ok(port) => (host, port),
                    Err(_) => (rest, default_ssh_port()),
                },
                None => (rest, default_ssh_port()),
            };
            SshJumpHost {
                host: host.to_string(),
                user,
                port,
                identity_file: identities.get(idx).and_then(|id| non_empty(Some(id))),
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "config_file_tests.rs"]
mod tests;
