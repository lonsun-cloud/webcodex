//! Desktop owns desired MCP configuration. Runner TOML is only a materialization.
//! Private immutable environment blobs are committed before their manifest reference;
//! a failed/interrupted manifest replacement leaves the previous configuration valid.
use crate::error::{DesktopError, DesktopResult};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use webcodex_core::mcp_gateway::{
    validate_provider_name, MCP_GATEWAY_MAX_ENV_MAPPINGS, MCP_GATEWAY_MAX_PROVIDERS,
};

const MAX_BYTES: u64 = 1024 * 1024;
const MAX_SAVED_PROVIDERS: usize = 64;
const MAX_MANAGED_IDS: usize = 4096;
pub use webcodex_runner_config::DESKTOP_MCP_ENV_PREFIX as PRIVATE_ENV_PREFIX;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderScope {
    Runner,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct McpProviderProfile {
    pub id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub enabled: bool,
    pub scope: ProviderScope,
    pub env_keys: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredProvider {
    profile: McpProviderProfile,
    secret_ref: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    revision: u64,
    profiles: Vec<StoredProvider>,
    /// Tombstones let removal reconcile an older A/B slot without deleting operator entries.
    managed_ids: BTreeSet<String>,
}
impl Default for Manifest {
    fn default() -> Self {
        Self {
            schema_version: 1,
            revision: 0,
            profiles: Vec::new(),
            managed_ids: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpProvidersSnapshot {
    pub revision: u64,
    pub profiles: Vec<McpProviderProfile>,
    pub restart_required: bool,
    pub config_error: bool,
    pub max_enabled: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpProviderRequest {
    pub id: Option<String>,
    pub expected_revision: u64,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub enabled: bool,
    /// Values are write-only; null retains the selected provider's existing value.
    /// Omitted keys remove the credential. Empty strings are intentional values.
    pub env: BTreeMap<String, Option<String>>,
}

#[derive(Clone)]
pub struct McpProviderStore {
    root: PathBuf,
    manifest: Manifest,
    original: Option<Vec<u8>>,
    invalid: bool,
}

impl McpProviderStore {
    pub fn load(root: &Path) -> Self {
        let mut store = Self {
            root: root.to_path_buf(),
            manifest: Manifest::default(),
            original: None,
            invalid: false,
        };
        let result = (|| -> DesktopResult<()> {
            store.original = read_optional(&store.manifest_path())?;
            if let Some(bytes) = &store.original {
                store.manifest = serde_json::from_slice(bytes).map_err(|_| invalid())?;
                validate_manifest(&store.manifest)?;
                for provider in &store.manifest.profiles {
                    store.credentials(provider)?;
                }
            }
            Ok(())
        })();
        store.invalid = result.is_err();
        store
    }

    fn manifest_path(&self) -> PathBuf {
        self.root.join("mcp-providers.json")
    }
    fn secret_path(&self, reference: &str) -> PathBuf {
        self.root
            .join("secrets/mcp-providers")
            .join(format!("{reference}.json"))
    }
    pub fn revision(&self) -> u64 {
        self.manifest.revision
    }
    pub fn snapshot(&self, applied_revision: Option<u64>) -> McpProvidersSnapshot {
        McpProvidersSnapshot {
            revision: self.manifest.revision,
            profiles: if self.invalid {
                Vec::new()
            } else {
                self.manifest
                    .profiles
                    .iter()
                    .map(|p| p.profile.clone())
                    .collect()
            },
            restart_required: !self.invalid
                && self.manifest.revision != 0
                && applied_revision != Some(self.manifest.revision),
            config_error: self.invalid,
            max_enabled: MCP_GATEWAY_MAX_PROVIDERS,
        }
    }
    pub fn profiles(&self) -> DesktopResult<Vec<McpProviderProfile>> {
        if self.invalid {
            return Err(invalid());
        }
        Ok(self
            .manifest
            .profiles
            .iter()
            .map(|p| p.profile.clone())
            .collect())
    }
    pub fn managed_ids(&self) -> &BTreeSet<String> {
        &self.manifest.managed_ids
    }

    pub fn update(&mut self, request: McpProviderRequest) -> DesktopResult<String> {
        self.check_revision(request.expected_revision)?;
        let previous = request
            .id
            .as_ref()
            .and_then(|id| self.manifest.profiles.iter().find(|p| &p.profile.id == id));
        if request.id.is_some() && previous.is_none() {
            return Err(invalid());
        }
        let id = request
            .id
            .unwrap_or_else(|| format!("desktop-mcp-{}", uuid::Uuid::new_v4().simple()));
        let profile = McpProviderProfile {
            id: id.clone(),
            name: request.name.trim().into(),
            command: request.command.trim().into(),
            args: request.args,
            cwd: request
                .cwd
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            enabled: request.enabled,
            scope: ProviderScope::Runner,
            env_keys: request.env.keys().cloned().collect(),
        };
        validate_profile(&profile)?;
        // Resolve executables without running them. Names such as npx are allowed in
        // desired state; the Runner contract receives an absolute host-local path.
        if profile.enabled {
            resolve_executable(&profile.command)?;
        }
        let old_values = previous
            .map(|p| self.credentials(p))
            .transpose()?
            .unwrap_or_default();
        let mut values = BTreeMap::new();
        for (key, value) in request.env {
            let value = value
                .or_else(|| old_values.get(&key).cloned())
                .ok_or_else(invalid)?;
            if value.len() > 8192 || value.contains('\0') {
                return Err(invalid());
            }
            values.insert(key, value);
        }
        let secret_ref = (!values.is_empty()).then(|| uuid::Uuid::new_v4().to_string());
        let next_provider = StoredProvider {
            profile,
            secret_ref: secret_ref.clone(),
        };
        let mut next = self.manifest.clone();
        match next.profiles.iter().position(|p| p.profile.id == id) {
            Some(index) => next.profiles[index] = next_provider,
            None => next.profiles.push(next_provider),
        }
        next.managed_ids.insert(id.clone());
        next.revision = next.revision.checked_add(1).ok_or_else(invalid)?;
        validate_manifest(&next)?;
        // Fence before creating a private candidate, then again at manifest commit.
        self.verify_disk()?;
        if let Some(reference) = &secret_ref {
            self.write_credentials(reference, &values)?;
        }
        self.persist(next)?;
        Ok(id)
    }

    pub fn remove(&mut self, id: &str, expected_revision: u64) -> DesktopResult<()> {
        self.check_revision(expected_revision)?;
        let mut next = self.manifest.clone();
        let index = next
            .profiles
            .iter()
            .position(|p| p.profile.id == id)
            .ok_or_else(invalid)?;
        next.profiles.remove(index);
        next.revision = next.revision.checked_add(1).ok_or_else(invalid)?;
        self.persist(next)
    }

    pub fn apply_to_command(&self, command: &mut Command) -> DesktopResult<()> {
        if self.invalid {
            return Err(invalid());
        }
        // Prevent inherited Desktop-MCP values from another runtime from becoming
        // this Runner's credential source. Injection is scoped to the selected profiles.
        for (name, _) in std::env::vars_os() {
            if name
                .to_string_lossy()
                .to_ascii_uppercase()
                .starts_with(PRIVATE_ENV_PREFIX)
            {
                command.env_remove(name);
            }
        }
        command.env("PATH", provider_path()?);
        for provider in self.manifest.profiles.iter().filter(|p| p.profile.enabled) {
            let values = self.credentials(provider)?;
            for (index, key) in provider.profile.env_keys.iter().enumerate() {
                command.env(
                    env_source(&provider.profile.id, index),
                    values.get(key).ok_or_else(invalid)?,
                );
            }
        }
        Ok(())
    }

    fn credentials(&self, provider: &StoredProvider) -> DesktopResult<BTreeMap<String, String>> {
        let Some(reference) = &provider.secret_ref else {
            return if provider.profile.env_keys.is_empty() {
                Ok(BTreeMap::new())
            } else {
                Err(invalid())
            };
        };
        self.verify_secret_directory()?;
        let bytes = read_optional(&self.secret_path(reference))?.ok_or_else(invalid)?;
        let values: BTreeMap<String, String> =
            serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if values.keys().cloned().collect::<Vec<_>>() != provider.profile.env_keys
            || values.values().any(|v| v.len() > 8192 || v.contains('\0'))
        {
            return Err(invalid());
        }
        Ok(values)
    }
    fn write_credentials(
        &self,
        reference: &str,
        values: &BTreeMap<String, String>,
    ) -> DesktopResult<()> {
        let parent = self.root.join("secrets/mcp-providers");
        fs::create_dir_all(&parent).map_err(|_| invalid())?;
        self.verify_secret_directory()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            fs::set_permissions(&parent, fs::Permissions::from_mode(0o700))
                .map_err(|_| invalid())?;
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(self.secret_path(reference))
                .map_err(|_| invalid())?;
            let bytes = serde_json::to_vec(values).map_err(|_| invalid())?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| invalid())?;
            File::open(parent)
                .and_then(|dir| dir.sync_all())
                .map_err(|_| invalid())?;
        }
        #[cfg(not(unix))]
        {
            let bytes = serde_json::to_vec(values).map_err(|_| invalid())?;
            crate::state::write_atomic_file(&self.secret_path(reference), &bytes)
                .map_err(|_| invalid())?;
        }
        Ok(())
    }
    fn verify_secret_directory(&self) -> DesktopResult<()> {
        for path in [
            self.root.join("secrets"),
            self.root.join("secrets/mcp-providers"),
        ] {
            if !fs::symlink_metadata(path)
                .map_err(|_| invalid())?
                .file_type()
                .is_dir()
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
    fn check_revision(&self, revision: u64) -> DesktopResult<()> {
        if self.invalid {
            return Err(invalid());
        }
        if revision != self.manifest.revision {
            return Err(changed());
        }
        Ok(())
    }
    fn verify_disk(&self) -> DesktopResult<()> {
        if read_optional(&self.manifest_path())? != self.original {
            return Err(changed());
        }
        Ok(())
    }
    fn persist(&mut self, next: Manifest) -> DesktopResult<()> {
        validate_manifest(&next)?;
        let bytes = serde_json::to_vec_pretty(&next).map_err(|_| invalid())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(invalid());
        }
        crate::state::write_atomic_file_with_hook(&self.manifest_path(), &bytes, |_| {
            self.verify_disk()
                .map_err(|_| std::io::ErrorKind::WouldBlock.into())
        })
        .map_err(|_| changed())?;
        let previous = std::mem::replace(&mut self.manifest, next);
        self.original = Some(bytes);
        // Delete only references retired by our committed transaction, never a
        // directory sweep that could race another writer's unpublished candidate.
        for reference in previous
            .profiles
            .iter()
            .filter_map(|p| p.secret_ref.as_ref())
        {
            if !self
                .manifest
                .profiles
                .iter()
                .any(|p| p.secret_ref.as_ref() == Some(reference))
            {
                let _ = fs::remove_file(self.secret_path(reference));
            }
        }
        Ok(())
    }
}

pub fn bootstrap_environment(keys: &[String]) -> Vec<&'static str> {
    [
        "PATH",
        "HOME",
        "TMPDIR",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "TEMP",
        "TMP",
        "SYSTEMROOT",
        "COMSPEC",
        "PATHEXT",
    ]
    .into_iter()
    .filter(|name| {
        (*name == "PATH" || std::env::var_os(name).is_some())
            && !keys.iter().any(|key| {
                if cfg!(windows) {
                    key.eq_ignore_ascii_case(name)
                } else {
                    key == name
                }
            })
    })
    .collect()
}

pub fn env_source(id: &str, index: usize) -> String {
    format!(
        "{PRIVATE_ENV_PREFIX}{}_{}",
        id.replace('-', "_").to_ascii_uppercase(),
        index
    )
}

pub fn provider_path() -> DesktopResult<std::ffi::OsString> {
    let mut paths: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p)
                .filter(|p| p.is_absolute())
                .collect()
        })
        .unwrap_or_default();
    #[cfg(unix)]
    {
        paths
            .extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from));
        if let Some(home) = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
        {
            paths.push(home.join(".local/bin"));
        }
    }
    let mut unique = BTreeSet::new();
    paths.retain(|p| unique.insert(p.clone()));
    std::env::join_paths(paths).map_err(|_| invalid())
}

pub fn resolve_executable(command: &str) -> DesktopResult<PathBuf> {
    let path = Path::new(command);
    if path.is_absolute() && executable_file(path) {
        return Ok(path.to_path_buf());
    }
    if path.components().count() != 1 {
        return Err(command_unavailable());
    }
    for parent in std::env::split_paths(&provider_path()?) {
        let candidate = parent.join(command);
        if executable_file(&candidate) {
            return Ok(candidate);
        }
        #[cfg(windows)]
        for extension in ["exe", "com", "cmd", "bat"] {
            let candidate = parent.join(format!("{command}.{extension}"));
            if executable_file(&candidate) {
                return Ok(candidate);
            }
        }
    }
    Err(command_unavailable())
}
fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}
fn validate_profile(p: &McpProviderProfile) -> DesktopResult<()> {
    let suffix = p.id.strip_prefix("desktop-mcp-").ok_or_else(invalid)?;
    if suffix.len() != 32
        || !suffix
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || validate_provider_name(&p.name).is_err()
        || p.command.is_empty()
        || p.command.len() > 1024
        || p.command.contains(['\0', '\n', '\r'])
        || p.args.len() > 64
        || p.args.iter().any(|s| s.len() > 4096 || s.contains('\0'))
        || p.args.iter().map(|s| s.len() + 1).sum::<usize>() > 16 * 1024
        || p.env_keys.len() > 64
    {
        return Err(invalid());
    }
    if let Some(cwd) = &p.cwd {
        if cwd.len() > 4096
            || !Path::new(cwd).is_absolute()
            || cwd.contains(['\0', '\n', '\r'])
            || webcodex_runner_config::paths::validate_project_path_ingress(Path::new(cwd)).is_err()
        {
            return Err(invalid());
        }
    }
    let mut keys = BTreeSet::new();
    for key in &p.env_keys {
        let normalized = key.to_ascii_uppercase();
        if key.is_empty()
            || key.len() > 128
            || !key.bytes().enumerate().all(|(index, b)| {
                b == b'_' || b.is_ascii_alphabetic() || (index > 0 && b.is_ascii_digit())
            })
            || (normalized.starts_with("WEBCODEX_") || normalized == "AUTHORIZATION")
            || !keys.insert(if cfg!(windows) {
                normalized
            } else {
                key.clone()
            })
        {
            return Err(invalid());
        }
    }
    if p.env_keys.windows(2).any(|pair| pair[0] >= pair[1])
        || p.env_keys.len() + bootstrap_environment(&p.env_keys).len()
            > MCP_GATEWAY_MAX_ENV_MAPPINGS
    {
        return Err(invalid());
    }
    Ok(())
}
fn validate_manifest(manifest: &Manifest) -> DesktopResult<()> {
    if manifest.schema_version != 1
        || manifest.profiles.len() > MAX_SAVED_PROVIDERS
        || manifest.managed_ids.len() > MAX_MANAGED_IDS
        || manifest
            .profiles
            .iter()
            .filter(|p| p.profile.enabled)
            .count()
            > MCP_GATEWAY_MAX_PROVIDERS
    {
        return Err(invalid());
    }
    let mut ids = BTreeSet::new();
    for provider in &manifest.profiles {
        validate_profile(&provider.profile)?;
        if !ids.insert(&provider.profile.id) || !manifest.managed_ids.contains(&provider.profile.id)
        {
            return Err(invalid());
        }
        match &provider.secret_ref {
            Some(reference)
                if uuid::Uuid::parse_str(reference)
                    .is_ok_and(|id| id.to_string() == *reference)
                    && !provider.profile.env_keys.is_empty() => {}
            None if provider.profile.env_keys.is_empty() => {}
            _ => return Err(invalid()),
        }
    }
    for id in &manifest.managed_ids {
        let Some(suffix) = id.strip_prefix("desktop-mcp-") else {
            return Err(invalid());
        };
        if suffix.len() != 32
            || !suffix
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(invalid());
        }
    }
    Ok(())
}
fn read_optional(path: &Path) -> DesktopResult<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(invalid()),
    };
    if !metadata.file_type().is_file() || metadata.len() > MAX_BYTES {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| invalid())?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(invalid());
    }
    Ok(Some(bytes))
}
pub fn invalid() -> DesktopError {
    DesktopError::new("mcp_provider_invalid", "MCP Provider settings are invalid or unavailable", "Check the name, command, arguments and environment fields. Saved private values are never returned.")
}
fn changed() -> DesktopError {
    DesktopError::new("mcp_provider_changed", "MCP Provider settings changed or could not be saved", "Refresh MCP Providers before saving again; the previous valid configuration is retained on an interrupted write.")
}
fn command_unavailable() -> DesktopError {
    DesktopError::new(
        "mcp_command_unavailable",
        "The MCP Provider command is not available",
        "Install the executable or choose its absolute path. Commands are not executed by a shell.",
    )
}

#[cfg(test)]
mod tests;
