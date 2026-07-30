use std::collections::HashMap;
use std::fs;
use std::io::{ErrorKind, Write as _};
use std::ops::Range;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use super::util::{SshHostProfile, SshJumpHost, default_ssh_port};

const WARP_ID_PREFIX: &str = "# warp-id:";
const WARP_TAGS_PREFIX: &str = "# warp-tags:";
const WARP_JUMP_IDENTITIES_PREFIX: &str = "# warp-jump-identities:";
const WARP_JUMP_SOURCES_PREFIX: &str = "# warp-jump-sources:";
const BACKUP_SUFFIX: &str = ".warp.bak";
const TEMP_SUFFIX: &str = ".warp.tmp";
const DEFAULT_INDENT: &str = "    ";
const LF: &str = "\n";
const CRLF: &str = "\r\n";
const HOST_PATTERN_CHARS: [char; 3] = ['*', '?', '!'];
const STANZA_BOUNDARY_KEYWORDS: [&str; 2] = ["Match", "Include"];
const PRIVATE_FILE_MODE: u32 = 0o600;
const PRIVATE_DIR_MODE: u32 = 0o700;
const DERIVED_ID_NAMESPACE: &str = "dev.warp.ssh-config.hand-written-host:";
const UUID_BYTE_COUNT: usize = 16;
const UUID_VERSION_BYTE: usize = 6;
const UUID_VARIANT_BYTE: usize = 8;
const UUID_VERSION_MASK: u8 = 0x0f;
const UUID_VERSION_CUSTOM: u8 = 0x80;
const UUID_VARIANT_MASK: u8 = 0x3f;
const UUID_VARIANT_RFC: u8 = 0x80;

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
    jump_sources: Vec<Option<Uuid>>,
    alias: String,
    meta: Vec<String>,
    meta_rewritten: bool,
    header: String,
    body: Vec<String>,
    eol: String,
}

/// The config file as an ordered list of things we rewrite (host blocks) and things we don't.
#[derive(Clone, Debug)]
enum Segment {
    /// Anything outside a Host stanza: global options, comments, blank lines, Include/Match.
    Raw(String),
    Host(HostBlock),
}

#[derive(Clone, Debug)]
struct ConfigFile {
    segments: Vec<Segment>,
    eol: String,
}

enum MetaComment {
    Id(Uuid),
    Tags(Vec<String>),
    JumpIdentities(Vec<String>),
    JumpSources(Vec<Option<Uuid>>),
}

#[derive(Default)]
struct PendingMeta {
    lines: Vec<String>,
    id: Option<Uuid>,
    tags: Vec<String>,
    jump_identities: Vec<String>,
    jump_sources: Vec<Option<Uuid>>,
}

impl PendingMeta {
    fn push(&mut self, comment: MetaComment, line: &str) {
        match comment {
            MetaComment::Id(id) => self.id = Some(id),
            MetaComment::Tags(tags) => self.tags = tags,
            MetaComment::JumpIdentities(identities) => self.jump_identities = identities,
            MetaComment::JumpSources(sources) => self.jump_sources = sources,
        }
        self.lines.push(line.to_string());
    }

    fn flush(&mut self, current: &mut Option<HostBlock>, segments: &mut Vec<Segment>) {
        for line in std::mem::take(self).lines {
            match current.as_mut() {
                Some(block) => block.body.push(line),
                None => segments.push(Segment::Raw(line)),
            }
        }
    }
}

fn parse(contents: &str) -> ConfigFile {
    let eol = detect_eol(contents).to_string();
    let mut segments: Vec<Segment> = Vec::new();
    let mut pending = PendingMeta::default();
    let mut current: Option<HostBlock> = None;

    for line in contents.split_inclusive('\n') {
        let trimmed = line.trim();

        if let Some(comment) = parse_meta_comment(trimmed) {
            pending.push(comment, line);
            continue;
        }

        if let Some(alias) = parse_host_header(trimmed) {
            if let Some(block) = current.take() {
                segments.push(Segment::Host(block));
            }
            let meta = std::mem::take(&mut pending);
            current = Some(HostBlock {
                warp_id: meta.id,
                tags: meta.tags,
                jump_identities: meta.jump_identities,
                jump_sources: meta.jump_sources,
                alias,
                meta: meta.lines,
                meta_rewritten: false,
                header: line.to_string(),
                body: Vec::new(),
                eol: eol.clone(),
            });
            continue;
        }

        pending.flush(&mut current, &mut segments);

        if is_stanza_boundary(trimmed) {
            if let Some(block) = current.take() {
                segments.push(Segment::Host(block));
            }
            segments.push(Segment::Raw(line.to_string()));
            continue;
        }

        match current.as_mut() {
            Some(block) => block.body.push(line.to_string()),
            None => segments.push(Segment::Raw(line.to_string())),
        }
    }

    pending.flush(&mut current, &mut segments);
    if let Some(block) = current.take() {
        segments.push(Segment::Host(block));
    }
    ConfigFile { segments, eol }
}

fn serialize(config: &ConfigFile) -> String {
    let mut out = String::new();
    for segment in &config.segments {
        match segment {
            Segment::Raw(line) => out.push_str(line),
            Segment::Host(block) => {
                for line in block.meta_lines() {
                    out.push_str(&line);
                }
                out.push_str(&block.header);
                for line in &block.body {
                    out.push_str(line);
                }
            }
        }
    }
    out
}

impl ConfigFile {
    fn host_block_position(&self, predicate: impl Fn(&HostBlock) -> bool) -> Option<usize> {
        self.segments
            .iter()
            .position(|segment| matches!(segment, Segment::Host(block) if predicate(block)))
    }

    fn push_profile_block(&mut self, profile: &SshHostProfile) -> Result<()> {
        let block = HostBlock::new_for(profile, &self.eol)?;
        self.terminate_last_line();
        let ends_with_blank_line =
            matches!(self.segments.last(), Some(Segment::Raw(line)) if line.trim().is_empty());
        if !self.segments.is_empty() && !ends_with_blank_line {
            self.segments.push(Segment::Raw(self.eol.clone()));
        }
        self.segments.push(Segment::Host(block));
        Ok(())
    }

    fn terminate_last_line(&mut self) {
        let eol = self.eol.clone();
        match self.segments.last_mut() {
            Some(Segment::Raw(line)) => terminate_line(line, &eol),
            Some(Segment::Host(block)) => block.terminate_last_line(),
            None => {}
        }
    }
}

impl HostBlock {
    fn is_pattern(&self) -> bool {
        alias_is_pattern(&self.alias)
    }

    fn directive(&self, name: &str) -> Option<String> {
        self.body.iter().find_map(|line| {
            let (key, value) = split_directive(line)?;
            key.eq_ignore_ascii_case(name).then_some(value)
        })
    }

    /// Replace a directive's value in place, keeping its inline comment; append if absent; drop if `None`.
    fn set_directive(&mut self, name: &str, value: Option<impl AsRef<str>>) {
        let value = value.as_ref().map(AsRef::as_ref);
        let existing = self.body.iter().enumerate().find_map(|(idx, line)| {
            let directive = parse_directive(line)?;
            let (_, range) = directive.value?;
            directive
                .key
                .eq_ignore_ascii_case(name)
                .then_some((idx, range))
        });

        match (existing, value) {
            (Some((idx, range)), Some(value)) => {
                let replaced = replace_range(&self.body[idx], range, value);
                self.body[idx] = replaced;
            }
            (Some((idx, _)), None) => {
                self.body.remove(idx);
            }
            (None, Some(value)) => {
                let indent = self.indent();
                let insert_at = self
                    .body
                    .iter()
                    .rposition(|line| !line.trim().is_empty())
                    .map(|idx| idx + 1)
                    .unwrap_or(0);
                self.insert_body_line(insert_at, format!("{indent}{name} {value}"));
            }
            (None, None) => {}
        }
    }

    fn set_alias(&mut self, alias: &str) {
        if self.alias == alias {
            return;
        }
        self.alias = alias.to_string();
        let value_range = parse_directive(&self.header)
            .and_then(|directive| directive.value)
            .map(|(_, range)| range);
        self.header = match value_range {
            Some(range) => replace_range(&self.header, range, alias),
            None => format!("Host {alias}{}", self.eol),
        };
    }

    fn insert_body_line(&mut self, at: usize, text: String) {
        if at == self.body.len() {
            self.terminate_last_line();
        }
        self.body.insert(at, format!("{text}{}", self.eol));
    }

    fn terminate_last_line(&mut self) {
        let eol = self.eol.clone();
        match self.body.last_mut() {
            Some(line) => terminate_line(line, &eol),
            None => terminate_line(&mut self.header, &eol),
        }
    }

    fn meta_lines(&self) -> Vec<String> {
        if !self.meta_rewritten {
            return self.meta.clone();
        }

        let mut lines = Vec::new();
        if let Some(id) = self.warp_id {
            lines.push(format!("{WARP_ID_PREFIX} {id}{}", self.eol));
        }
        if !self.tags.is_empty() {
            lines.push(format!(
                "{WARP_TAGS_PREFIX} {}{}",
                self.tags.join(","),
                self.eol
            ));
        }
        if !self.jump_identities.is_empty() {
            lines.push(format!(
                "{WARP_JUMP_IDENTITIES_PREFIX} {}{}",
                self.jump_identities.join(","),
                self.eol
            ));
        }
        if !self.jump_sources.is_empty() {
            lines.push(format!(
                "{WARP_JUMP_SOURCES_PREFIX} {}{}",
                format_jump_sources(&self.jump_sources),
                self.eol
            ));
        }
        lines
    }

    fn indent(&self) -> String {
        self.body
            .iter()
            .find(|line| !line.trim().is_empty() && line.starts_with(char::is_whitespace))
            .map(|line| {
                line.chars()
                    .take_while(|c| c.is_whitespace())
                    .collect::<String>()
            })
            .unwrap_or_else(|| DEFAULT_INDENT.to_string())
    }

    fn to_profile(&self, alias_ordinal: usize) -> Result<SshHostProfile> {
        let host = self
            .directive("HostName")
            .unwrap_or_else(|| self.alias.clone());
        let port = match self.directive("Port") {
            Some(value) => value.parse().with_context(|| {
                format!(
                    "`Port {value}` in the ssh config stanza `Host {}` is not a port number",
                    self.alias
                )
            })?,
            None => default_ssh_port(),
        };
        let jump_hosts = match self.directive("ProxyJump") {
            Some(value) => parse_proxy_jump(&value, &self.jump_identities, &self.jump_sources)
                .with_context(|| {
                format!(
                    "`ProxyJump {value}` in the ssh config stanza `Host {}` is not a jump host chain",
                    self.alias
                )
            })?,
            None => Vec::new(),
        };

        Ok(SshHostProfile {
            id: self
                .warp_id
                .unwrap_or_else(|| derived_profile_id(&self.alias, alias_ordinal)),
            name: self.alias.clone(),
            host,
            user: self.directive("User"),
            port,
            identity_file: self.directive("IdentityFile"),
            jump_hosts,
            tags: self.tags.clone(),
        })
    }

    fn apply_profile(&mut self, profile: &SshHostProfile) -> Result<()> {
        validate_profile(profile)?;
        if self.is_pattern() {
            anyhow::bail!(
                "`Host {}` matches hosts by pattern, so it cannot be written as a profile",
                self.alias
            );
        }
        if alias_is_pattern(&profile.name) {
            anyhow::bail!(
                "SSH profile name `{}` would turn the stanza into a host pattern",
                profile.name
            );
        }

        self.warp_id = Some(profile.id);
        self.tags = profile.tags.clone();
        self.meta_rewritten = true;
        self.set_alias(&profile.name);

        self.set_directive("HostName", Some(&profile.host));
        self.set_directive("User", non_empty(profile.user.as_deref()));
        let port_already_written = self.directive("Port").is_some();
        self.set_directive(
            "Port",
            (profile.port != default_ssh_port() || port_already_written)
                .then(|| profile.port.to_string()),
        );
        self.set_directive("IdentityFile", non_empty(profile.identity_file.as_deref()));

        if profile.jump_hosts.is_empty() {
            self.set_directive("ProxyJump", None::<&str>);
            self.jump_identities.clear();
            self.jump_sources.clear();
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
            self.jump_sources = profile
                .jump_hosts
                .iter()
                .map(|jump| jump.source_profile_id)
                .collect();
            if self.jump_sources.iter().all(Option::is_none) {
                self.jump_sources.clear();
            }
        }
        Ok(())
    }

    fn new_for(profile: &SshHostProfile, eol: &str) -> Result<Self> {
        validate_profile(profile)?;
        let mut block = HostBlock {
            warp_id: None,
            tags: Vec::new(),
            jump_identities: Vec::new(),
            jump_sources: Vec::new(),
            alias: profile.name.clone(),
            meta: Vec::new(),
            meta_rewritten: true,
            header: format!("Host {}{eol}", profile.name),
            body: Vec::new(),
            eol: eol.to_string(),
        };
        block.apply_profile(profile)?;
        Ok(block)
    }
}

/// Read every `Host` stanza in `~/.ssh/config` as a profile. Hand-written hosts are included.
pub fn load_profiles() -> Result<Vec<SshHostProfile>> {
    let Some(path) = default_ssh_config_path() else {
        return Ok(Vec::new());
    };
    load_profiles_at(&path)
}

fn load_profiles_at(path: &Path) -> Result<Vec<SshHostProfile>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;

    let config = parse(&contents);
    let mut alias_ordinals: HashMap<&str, usize> = HashMap::new();
    config
        .segments
        .iter()
        .filter_map(|segment| match segment {
            Segment::Host(block) if !block.is_pattern() => {
                let ordinal = alias_ordinals.entry(&block.alias).or_default();
                let profile = block.to_profile(*ordinal);
                *ordinal += 1;
                Some(profile)
            }
            _ => None,
        })
        .collect()
}

/// Ids of the stanzas Warp wrote (those carrying `# warp-id:`); hand-written hosts are absent.
pub fn load_owned_profile_ids() -> Result<Vec<Uuid>> {
    let Some(path) = default_ssh_config_path() else {
        return Ok(Vec::new());
    };
    load_owned_profile_ids_at(&path)
}

fn load_owned_profile_ids_at(path: &Path) -> Result<Vec<Uuid>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;

    Ok(parse(&contents)
        .segments
        .iter()
        .filter_map(|segment| match segment {
            Segment::Host(block) if !block.is_pattern() => block.warp_id,
            _ => None,
        })
        .collect())
}

/// Create or update the stanza for `profile`, leaving everything else in the file untouched.
pub fn upsert_profile(profile: &SshHostProfile) -> Result<()> {
    let path = config_path()?;
    upsert_profile_at(&path, profile)
}

fn upsert_profile_at(path: &Path, profile: &SshHostProfile) -> Result<()> {
    validate_profile(profile)?;
    edit(path, |config| {
        let owned = config.host_block_position(|block| block.warp_id == Some(profile.id));
        let unclaimed = || {
            config.host_block_position(|block| {
                block.warp_id.is_none() && !block.is_pattern() && block.alias == profile.name
            })
        };

        let target = owned.or_else(unclaimed);
        match target {
            Some(idx) => {
                let Segment::Host(block) = &mut config.segments[idx] else {
                    anyhow::bail!(
                        "Failed to locate the Host stanza for profile {}",
                        profile.id
                    );
                };
                block.apply_profile(profile)
            }
            None => config.push_profile_block(profile),
        }
    })
}

/// Update the `# warp-id:` of the Warp-owned stanzas, one id per profile stanza in file order.
pub fn rewrite_profile_ids(ids: &[Uuid]) -> Result<()> {
    let path = config_path()?;
    rewrite_profile_ids_at(&path, ids)
}

fn rewrite_profile_ids_at(path: &Path, ids: &[Uuid]) -> Result<()> {
    edit(path, |config| {
        let mut blocks = config
            .segments
            .iter_mut()
            .filter_map(|segment| match segment {
                Segment::Host(block) if !block.is_pattern() => Some(block),
                _ => None,
            })
            .collect::<Vec<_>>();
        if blocks.len() != ids.len() {
            anyhow::bail!(
                "{} now holds {} profile stanzas instead of {}; it changed while Warp was updating profile ids",
                path.display(),
                blocks.len(),
                ids.len()
            );
        }

        for (block, id) in blocks.iter_mut().zip(ids) {
            if block.warp_id.is_none() || block.warp_id == Some(*id) {
                continue;
            }
            block.warp_id = Some(*id);
            block.meta_rewritten = true;
        }
        Ok(())
    })
}

/// Remove the stanza owned by `id`. Hand-written hosts (no warp id) are reported as an error.
pub fn remove_profile(id: Uuid) -> Result<()> {
    let path = config_path()?;
    remove_profile_at(&path, id)
}

fn remove_profile_at(path: &Path, id: Uuid) -> Result<()> {
    edit(path, |config| {
        let before = config.segments.len();
        config.segments.retain(
            |segment| !matches!(segment, Segment::Host(block) if block.warp_id == Some(id)),
        );
        if config.segments.len() == before {
            anyhow::bail!(
                "{} has no Warp-owned Host stanza for profile {id}; hand-written hosts have to be removed from the file by hand",
                path.display()
            );
        }
        Ok(())
    })
}

fn config_path() -> Result<PathBuf> {
    default_ssh_config_path().context("Could not determine the location of ~/.ssh/config")
}

/// Rewrite the config atomically when the contents change, keeping a `.warp.bak` of what Warp found.
fn edit(path: &Path, mutate: impl FnOnce(&mut ConfigFile) -> Result<()>) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        && !parent.exists()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
        set_mode(parent, PRIVATE_DIR_MODE)?;
    }

    let target = resolve_symlink(path)?;
    let contents = if target.exists() {
        fs::read_to_string(&target)
            .with_context(|| format!("Failed to read {}", target.display()))?
    } else {
        String::new()
    };

    let mut config = parse(&contents);
    mutate(&mut config)?;
    let updated = serialize(&config);
    if target.exists() && updated == contents {
        return Ok(());
    }

    let backup = sibling_path(path, BACKUP_SUFFIX);
    if target.exists() && !backup.exists() {
        fs::copy(&target, &backup)
            .with_context(|| format!("Failed to back up {}", target.display()))?;
    }

    write_atomically(&target, &updated)
}

fn write_atomically(path: &Path, contents: &str) -> Result<()> {
    let mode = existing_file_mode(path)?;
    let temp = sibling_path(path, TEMP_SUFFIX);
    if temp.exists() {
        fs::remove_file(&temp)
            .with_context(|| format!("Failed to remove the stale {}", temp.display()))?;
    }

    let result = write_temp_then_rename(&temp, path, contents, mode);
    if result.is_err()
        && let Err(err) = fs::remove_file(&temp)
        && err.kind() != ErrorKind::NotFound
    {
        log::warn!(
            "Failed to remove {} after a failed write: {err}",
            temp.display()
        );
    }
    result
}

fn write_temp_then_rename(
    temp: &Path,
    path: &Path,
    contents: &str,
    mode: Option<u32>,
) -> Result<()> {
    let mut file = create_private_file(temp)?;
    file.write_all(contents.as_bytes())
        .with_context(|| format!("Failed to write {}", temp.display()))?;
    file.sync_all()
        .with_context(|| format!("Failed to flush {} to disk", temp.display()))?;
    drop(file);

    if let Some(mode) = mode {
        set_mode(temp, mode)?;
    }
    fs::rename(temp, path).with_context(|| format!("Failed to replace {}", path.display()))
}

fn resolve_symlink(path: &Path) -> Result<PathBuf> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::canonicalize(path)
            .with_context(|| format!("Failed to resolve the symlink {}", path.display())),
        Ok(_) => Ok(path.to_path_buf()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(path.to_path_buf()),
        Err(err) => Err(err).with_context(|| format!("Failed to inspect {}", path.display())),
    }
}

fn sibling_path(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

#[cfg(unix)]
fn create_private_file(path: &Path) -> Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(PRIVATE_FILE_MODE)
        .open(path)
        .with_context(|| format!("Failed to create {}", path.display()))
}

#[cfg(not(unix))]
fn create_private_file(path: &Path) -> Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("Failed to create {}", path.display()))
}

#[cfg(unix)]
fn existing_file_mode(path: &Path) -> Result<Option<u32>> {
    use std::os::unix::fs::PermissionsExt as _;
    match fs::metadata(path) {
        Ok(metadata) => Ok(Some(metadata.permissions().mode() & 0o7777)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("Failed to inspect {}", path.display())),
    }
}

#[cfg(not(unix))]
fn existing_file_mode(_path: &Path) -> Result<Option<u32>> {
    Ok(None)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .with_context(|| format!("Failed to set permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

fn validate_profile(profile: &SshHostProfile) -> Result<()> {
    validate_value("name", &profile.name)?;
    validate_value("host", &profile.host)?;
    if let Some(user) = &profile.user {
        validate_value("user", user)?;
    }
    if let Some(identity_file) = &profile.identity_file {
        validate_value("identity file", identity_file)?;
    }
    for tag in &profile.tags {
        validate_value("tag", tag)?;
    }
    for jump in &profile.jump_hosts {
        validate_value("jump host", &jump.host)?;
        if let Some(user) = &jump.user {
            validate_value("jump host user", user)?;
        }
        if let Some(identity_file) = &jump.identity_file {
            validate_value("jump host identity file", identity_file)?;
        }
    }
    Ok(())
}

fn validate_value(field: &str, value: &str) -> Result<()> {
    if let Some(control) = value.chars().find(|c| c.is_control()) {
        anyhow::bail!(
            "SSH profile {field} contains the control character U+{:04X}, which cannot be written to ~/.ssh/config",
            control as u32
        );
    }
    Ok(())
}

fn alias_is_pattern(alias: &str) -> bool {
    alias.contains(HOST_PATTERN_CHARS)
}

fn detect_eol(contents: &str) -> &'static str {
    let crlf = contents.matches(CRLF).count();
    let lf_only = contents.matches(LF).count() - crlf;
    if crlf > lf_only { CRLF } else { LF }
}

fn terminate_line(line: &mut String, eol: &str) {
    if !line.ends_with('\n') {
        line.push_str(eol);
    }
}

fn replace_range(line: &str, range: Range<usize>, value: &str) -> String {
    format!("{}{value}{}", &line[..range.start], &line[range.end..])
}

fn parse_meta_comment(trimmed: &str) -> Option<MetaComment> {
    if let Some(value) = strip_prefix_ci(trimmed, WARP_ID_PREFIX) {
        return match Uuid::parse_str(value.trim()) {
            Ok(id) => Some(MetaComment::Id(id)),
            Err(_) => {
                log::warn!("Keeping `{trimmed}` in the ssh config as a comment: not a profile id");
                None
            }
        };
    }
    if let Some(value) = strip_prefix_ci(trimmed, WARP_TAGS_PREFIX) {
        return Some(MetaComment::Tags(split_csv(value)));
    }
    if let Some(value) = strip_prefix_ci(trimmed, WARP_JUMP_SOURCES_PREFIX) {
        return match parse_jump_sources(value) {
            Some(sources) => Some(MetaComment::JumpSources(sources)),
            None => {
                log::warn!(
                    "Keeping `{trimmed}` in the ssh config as a comment: not a list of profile ids"
                );
                None
            }
        };
    }
    strip_prefix_ci(trimmed, WARP_JUMP_IDENTITIES_PREFIX)
        .map(|value| MetaComment::JumpIdentities(split_positional_csv(value)))
}

fn parse_jump_sources(value: &str) -> Option<Vec<Option<Uuid>>> {
    split_positional_csv(value)
        .into_iter()
        .map(|item| match item.is_empty() {
            true => Some(None),
            false => Uuid::parse_str(&item).ok().map(Some),
        })
        .collect()
}

fn format_jump_sources(sources: &[Option<Uuid>]) -> String {
    sources
        .iter()
        .map(|source| source.map(|id| id.to_string()).unwrap_or_default())
        .collect::<Vec<_>>()
        .join(",")
}

/// Hand-written stanzas carry no `# warp-id:`, so their id has to be derivable from the file alone.
fn derived_profile_id(alias: &str, alias_ordinal: usize) -> Uuid {
    let digest =
        Sha256::digest(format!("{DERIVED_ID_NAMESPACE}{alias}\0{alias_ordinal}").as_bytes());
    let mut bytes = [0u8; UUID_BYTE_COUNT];
    bytes.copy_from_slice(&digest[..UUID_BYTE_COUNT]);
    bytes[UUID_VERSION_BYTE] = (bytes[UUID_VERSION_BYTE] & UUID_VERSION_MASK) | UUID_VERSION_CUSTOM;
    bytes[UUID_VARIANT_BYTE] = (bytes[UUID_VARIANT_BYTE] & UUID_VARIANT_MASK) | UUID_VARIANT_RFC;
    Uuid::from_bytes(bytes)
}

fn parse_host_header(trimmed: &str) -> Option<String> {
    let (key, value) = split_directive(trimmed)?;
    key.eq_ignore_ascii_case("Host").then_some(value)
}

/// `Match`/`Include` end the preceding `Host` stanza and their bodies belong to no profile.
fn is_stanza_boundary(trimmed: &str) -> bool {
    parse_directive(trimmed).is_some_and(|directive| {
        STANZA_BOUNDARY_KEYWORDS
            .iter()
            .any(|keyword| directive.key.eq_ignore_ascii_case(keyword))
    })
}

struct DirectiveSpan<'a> {
    key: &'a str,
    value: Option<(&'a str, Range<usize>)>,
}

/// Split `Key value  # trailing comment` (or `Key=value`) into its keyword and the value's span.
fn parse_directive(line: &str) -> Option<DirectiveSpan<'_>> {
    let key_start = line.find(|c: char| !c.is_whitespace())?;
    if line[key_start..].starts_with('#') {
        return None;
    }
    let key_end = line[key_start..]
        .find(|c: char| c.is_whitespace() || c == '=')
        .map_or(line.len(), |offset| key_start + offset);
    let key = &line[key_start..key_end];

    let mut value_start = skip_whitespace(line, key_end);
    if line[value_start..].starts_with('=') {
        value_start = skip_whitespace(line, value_start + 1);
    }
    let content_end = line[value_start..]
        .find('#')
        .map_or(line.len(), |offset| value_start + offset);
    let value = line[value_start..content_end].trim_end();
    let value = (!value.is_empty()).then(|| (value, value_start..value_start + value.len()));

    Some(DirectiveSpan { key, value })
}

fn split_directive(line: &str) -> Option<(&str, String)> {
    let directive = parse_directive(line)?;
    let (value, _) = directive.value?;
    Some((directive.key, unquote(value).to_string()))
}

fn skip_whitespace(line: &str, from: usize) -> usize {
    line[from..]
        .find(|c: char| !c.is_whitespace())
        .map_or(line.len(), |offset| from + offset)
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
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

/// Per-hop metadata is positional, so empty entries have to survive the split.
fn split_positional_csv(value: &str) -> Vec<String> {
    if value.trim().is_empty() {
        return Vec::new();
    }
    value
        .split(',')
        .map(|item| item.trim().to_string())
        .collect()
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// `user@host:port,user2@[2001:db8::1]:port` — the standard ProxyJump chain format.
fn format_proxy_jump(jump_hosts: &[SshJumpHost]) -> String {
    jump_hosts
        .iter()
        .map(|jump| {
            let mut target = String::new();
            if let Some(user) = non_empty(jump.user.as_deref()) {
                target.push_str(&user);
                target.push('@');
            }
            let bracketed = jump.host.starts_with('[') && jump.host.ends_with(']');
            if jump.host.contains(':') && !bracketed {
                target.push('[');
                target.push_str(&jump.host);
                target.push(']');
            } else {
                target.push_str(&jump.host);
            }
            if jump.port != default_ssh_port() {
                target.push(':');
                target.push_str(&jump.port.to_string());
            }
            target
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_proxy_jump(
    value: &str,
    identities: &[String],
    sources: &[Option<Uuid>],
) -> Result<Vec<SshJumpHost>> {
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
            let (host, port) = split_jump_host_port(rest)?;
            Ok(SshJumpHost {
                source_profile_id: sources.get(idx).copied().flatten(),
                host,
                user,
                port,
                identity_file: identities.get(idx).and_then(|id| non_empty(Some(id))),
            })
        })
        .collect()
}

fn split_jump_host_port(rest: &str) -> Result<(String, u16)> {
    if let Some(inner) = rest.strip_prefix('[') {
        let Some((host, after)) = inner.split_once(']') else {
            anyhow::bail!("`{rest}` is missing the closing bracket of its address");
        };
        let port = match after.strip_prefix(':') {
            Some(port) => parse_port(port)?,
            None if after.is_empty() => default_ssh_port(),
            None => anyhow::bail!("`{rest}` has trailing characters after its address"),
        };
        return Ok((host.to_string(), port));
    }

    match rest.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => Ok((host.to_string(), parse_port(port)?)),
        _ => Ok((rest.to_string(), default_ssh_port())),
    }
}

fn parse_port(value: &str) -> Result<u16> {
    value
        .parse()
        .with_context(|| format!("`{value}` is not a port number"))
}

#[cfg(test)]
#[path = "config_file_tests.rs"]
mod tests;
