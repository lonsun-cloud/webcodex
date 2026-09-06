//! OpenRC Runner service backend (Alpine Linux and other musl distributions).
//!
//! OpenRC systems have no systemd: this module renders and manages an
//! `openrc-run` + `supervise-daemon` service while reusing the systemd
//! backend's atomic-write, symlink-preflight, and rollback machinery. The
//! systemd backend keeps its existing behavior and paths; both are selected
//! through `--service-manager auto|systemd|openrc`. OpenRC supports only the
//! system scope; there is no user-session OpenRC integration.

use std::path::{Path, PathBuf};

use super::service::{
    best_effort_execute, execute_required, install_error_with_rollback, preflight_service_path,
    push_rollback_error, restore_unit_file, write_text_file_atomic_mode, ExistingUnitKind,
    ProcessExecutor, ProcessInvocation, RealProcessExecutor, ServiceControl, SystemdStatus,
};
use super::system::discover_named_binary_absolute;
use crate::{RunnerInstallServiceOptions, ServiceScope};

pub(crate) const RUNNER_OPENRC_SERVICE: &str = "webcodex-runner";
pub(crate) const OPENRC_RUNNER_INIT_FILE: &str = "/etc/init.d/webcodex-runner";
const OPENRC_INIT_DIR: &str = "/etc/init.d";
const OPENRC_RUNLEVELS_DIR: &str = "/etc/runlevels";
const OPENRC_DEFAULT_RUNLEVEL: &str = "default";

/// Concrete service manager backend used for Runner service operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServiceManager {
    Systemd,
    Openrc,
}

impl ServiceManager {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Systemd => "systemd",
            Self::Openrc => "openrc",
        }
    }
}

/// `--service-manager` selection. `Auto` probes the host at execution time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServiceManagerSelection {
    Auto,
    Systemd,
    Openrc,
}

impl ServiceManagerSelection {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "auto" => Ok(Self::Auto),
            "systemd" => Ok(Self::Systemd),
            "openrc" => Ok(Self::Openrc),
            _ => Err("--service-manager must be 'auto', 'systemd' or 'openrc'".to_string()),
        }
    }
}

/// Host evidence consulted by [`select_service_manager`]. Kept as a plain
/// struct so detection stays unit-testable without touching the real host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ServiceManagerProbe {
    /// /run/openrc exists and is a real directory (OpenRC runtime state).
    pub(crate) openrc_runtime: bool,
    /// /run/systemd/system exists and is a real directory (running systemd).
    pub(crate) systemd_runtime: bool,
    pub(crate) rc_service: bool,
    pub(crate) rc_update: bool,
    pub(crate) systemctl: bool,
}

fn is_real_directory(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        .unwrap_or(false)
}

pub(crate) fn probe_service_manager() -> ServiceManagerProbe {
    ServiceManagerProbe {
        openrc_runtime: is_real_directory(Path::new("/run/openrc")),
        systemd_runtime: is_real_directory(Path::new("/run/systemd/system")),
        rc_service: discover_named_binary_absolute("rc-service").is_some(),
        rc_update: discover_named_binary_absolute("rc-update").is_some(),
        systemctl: discover_named_binary_absolute("systemctl").is_some(),
    }
}

pub(crate) fn select_service_manager(
    selection: ServiceManagerSelection,
    probe: &ServiceManagerProbe,
) -> Result<ServiceManager, String> {
    match selection {
        ServiceManagerSelection::Systemd => Ok(ServiceManager::Systemd),
        ServiceManagerSelection::Openrc => Ok(ServiceManager::Openrc),
        ServiceManagerSelection::Auto => detect_service_manager(probe),
    }
}

fn detect_service_manager(probe: &ServiceManagerProbe) -> Result<ServiceManager, String> {
    if probe.openrc_runtime && probe.systemd_runtime {
        return Err(
            "cannot reliably detect the service manager: both /run/openrc and /run/systemd/system exist; pass --service-manager systemd|openrc explicitly"
                .to_string(),
        );
    }
    if probe.openrc_runtime {
        if probe.rc_service && probe.rc_update {
            return Ok(ServiceManager::Openrc);
        }
        return Err(
            "detected an OpenRC runtime (/run/openrc) but rc-service or rc-update is missing from PATH; install openrc or pass --service-manager explicitly"
                .to_string(),
        );
    }
    if probe.systemd_runtime {
        return Ok(ServiceManager::Systemd);
    }
    let openrc_tools = probe.rc_service && probe.rc_update;
    match (probe.systemctl, openrc_tools) {
        (true, false) => Ok(ServiceManager::Systemd),
        (false, true) => Ok(ServiceManager::Openrc),
        (true, true) => Err(
            "cannot reliably detect the service manager: both systemctl and the OpenRC tools (rc-service, rc-update) are available, but neither /run/systemd/system nor /run/openrc exists; pass --service-manager explicitly"
                .to_string(),
        ),
        (false, false) => Err(
            "no supported service manager detected (looked for /run/openrc, /run/systemd/system, systemctl, rc-service and rc-update); pass --service-manager explicitly"
                .to_string(),
        ),
    }
}

/// Resolve a `--service-manager` selection to a concrete backend. Explicit
/// selections never probe the host; `auto` on non-Linux keeps the historical
/// systemd path, whose helpers degrade to "unknown" or the existing
/// Linux-only error exactly as before.
pub(crate) fn resolve_service_manager(
    selection: ServiceManagerSelection,
) -> Result<ServiceManager, String> {
    match selection {
        ServiceManagerSelection::Systemd => Ok(ServiceManager::Systemd),
        ServiceManagerSelection::Openrc if !cfg!(target_os = "linux") => {
            Err("OpenRC service management is supported only on Linux".to_string())
        }
        ServiceManagerSelection::Openrc => Ok(ServiceManager::Openrc),
        ServiceManagerSelection::Auto if !cfg!(target_os = "linux") => Ok(ServiceManager::Systemd),
        ServiceManagerSelection::Auto => {
            select_service_manager(selection, &probe_service_manager())
        }
    }
}

pub(crate) fn rc_service_path() -> Result<PathBuf, String> {
    if !cfg!(target_os = "linux") {
        return Err("OpenRC service management is supported only on Linux".to_string());
    }
    discover_named_binary_absolute("rc-service").ok_or_else(|| {
        "rc-service was not found in an absolute PATH entry; install openrc or use --service-manager systemd"
            .to_string()
    })
}

pub(crate) fn rc_update_path() -> Result<PathBuf, String> {
    if !cfg!(target_os = "linux") {
        return Err("OpenRC service management is supported only on Linux".to_string());
    }
    discover_named_binary_absolute("rc-update").ok_or_else(|| {
        "rc-update was not found in an absolute PATH entry; install openrc or use --service-manager systemd"
            .to_string()
    })
}

fn tail_path() -> Result<PathBuf, String> {
    if !cfg!(target_os = "linux") {
        return Err("OpenRC log access is supported only on Linux".to_string());
    }
    discover_named_binary_absolute("tail")
        .ok_or_else(|| "tail was not found in an absolute PATH entry".to_string())
}

pub(crate) fn default_openrc_init_file() -> PathBuf {
    PathBuf::from(OPENRC_RUNNER_INIT_FILE)
}

/// Service name derived from the init script file name.
pub(crate) fn openrc_service_name(service_file: &Path) -> String {
    service_file
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(RUNNER_OPENRC_SERVICE)
        .to_string()
}

/// Conventional log file written by supervise-daemon for this service.
pub(crate) fn openrc_log_file(service: &str) -> PathBuf {
    PathBuf::from(format!("/var/log/{service}.log"))
}

fn validate_openrc_identity(field: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("invalid OpenRC {field} value: cannot be empty"));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(format!(
            "invalid OpenRC {field} value: use only ASCII letters, digits, '_', '-' or '.'"
        ));
    }
    Ok(())
}

/// Validate a value embedded in a double-quoted POSIX shell context of the
/// rendered init script, including the `${VAR:-default}` override form.
fn openrc_shell_value<'a>(field: &str, value: &'a str) -> Result<&'a str, String> {
    if value.is_empty() {
        return Err(format!("invalid OpenRC {field} value: cannot be empty"));
    }
    if value
        .chars()
        .any(|ch| ch.is_control() || matches!(ch, '"' | '\\' | '$' | '`' | '}'))
    {
        return Err(format!(
            "invalid OpenRC {field} value: contains a character that is unsafe in an init script"
        ));
    }
    Ok(value)
}

fn openrc_shell_path<'a>(field: &str, path: &'a Path) -> Result<&'a str, String> {
    if !path.is_absolute() {
        return Err(format!(
            "invalid OpenRC {field} value: path must be absolute"
        ));
    }
    let value = path
        .to_str()
        .ok_or_else(|| format!("invalid OpenRC {field} value: path is not valid UTF-8"))?;
    openrc_shell_value(field, value)
}

/// `command` and `command_args` are word-split by openrc-run; reject
/// whitespace so the rendered command line cannot gain or lose arguments.
fn openrc_command_path<'a>(field: &str, path: &'a Path) -> Result<&'a str, String> {
    let value = openrc_shell_path(field, path)?;
    if value.chars().any(char::is_whitespace) {
        return Err(format!(
            "invalid OpenRC {field} value: whitespace is not supported in this path"
        ));
    }
    Ok(value)
}

/// Render the `/etc/init.d/<service>` openrc-run script. The script references
/// the Runner config via `--config`; it never contains tokens. Values are
/// embedded as `${VAR:-default}` so an optional `/etc/conf.d/<service>` file
/// can override them without regenerating the script. Syntax is POSIX/BusyBox
/// compatible: no Bash, systemctl, or journalctl.
pub(crate) fn render_runner_openrc_init(
    opts: &RunnerInstallServiceOptions,
    service_file: &Path,
) -> Result<String, String> {
    if opts.scope != ServiceScope::System {
        return Err(
            "OpenRC supports only --scope system; rerun as root or pass --scope system".to_string(),
        );
    }
    if opts.root_runner && !opts.allow_root_runner {
        return Err(
            "refusing to render a Runner that would run as root without --allow-root-runner"
                .to_string(),
        );
    }
    let service = openrc_service_name(service_file);
    validate_openrc_identity("service name", &service)?;
    let command = openrc_command_path("command", &opts.bin)?.to_string();
    let config = openrc_command_path("--config", &opts.config)?.to_string();
    let working_directory =
        openrc_shell_path("working directory", &opts.working_directory)?.to_string();
    let log_path = openrc_log_file(&service);
    let log_file = log_path
        .to_str()
        .ok_or_else(|| "invalid OpenRC log file value: path is not valid UTF-8".to_string())?
        .to_string();
    let command_user = match (&opts.user, &opts.group) {
        (Some(user), group) => {
            validate_openrc_identity("user", user)?;
            match group {
                Some(group) => {
                    validate_openrc_identity("group", group)?;
                    Some(format!("{user}:{group}"))
                }
                None => Some(user.clone()),
            }
        }
        (None, Some(_)) => {
            return Err("--group requires --user for an OpenRC service".to_string());
        }
        (None, None) => None,
    };

    let mut script = String::new();
    script.push_str("#!/sbin/openrc-run\n");
    script.push_str("# WebCodex Runner service for OpenRC (supervise-daemon).\n");
    script.push_str("# Rendered by `webcodex runner install --service-manager openrc`.\n");
    if opts.root_runner {
        script.push_str(
            "# WARNING: --allow-root-runner was explicitly accepted; project commands run as root.\n",
        );
    }
    script.push_str("# Secrets stay in the Runner config named by --config; none appear here.\n");
    script.push_str("# Optional overrides belong in /etc/conf.d/$RC_SVCNAME using the\n");
    script.push_str("# WEBCODEX_RUNNER_* variables below.\n\n");
    script.push_str(&format!("name=\"{service}\"\n"));
    script.push_str("description=\"WebCodex Runner\"\n\n");
    script.push_str("supervisor=\"supervise-daemon\"\n");
    script.push_str(&format!(
        "command=\"${{WEBCODEX_RUNNER_BIN:-{command}}}\"\n"
    ));
    script.push_str(&format!(
        "command_args=\"--config ${{WEBCODEX_RUNNER_CONFIG:-{config}}}\"\n"
    ));
    if let Some(command_user) = &command_user {
        script.push_str(&format!(
            "command_user=\"${{WEBCODEX_RUNNER_USER:-{command_user}}}\"\n"
        ));
    }
    script.push_str(&format!(
        "directory=\"${{WEBCODEX_RUNNER_WORKING_DIRECTORY:-{working_directory}}}\"\n"
    ));
    script.push_str("pidfile=\"/run/${RC_SVCNAME}.pid\"\n");
    script.push_str(&format!(
        "output_log=\"${{WEBCODEX_RUNNER_LOG:-{log_file}}}\"\n"
    ));
    script.push_str(&format!(
        "error_log=\"${{WEBCODEX_RUNNER_LOG:-{log_file}}}\"\n\n"
    ));
    script.push_str("extra_started_commands=\"reload\"\n\n");
    script.push_str("depend() {\n\tuse net\n\tafter net\n}\n\n");
    script.push_str(
        "reload() {\n\tebegin \"Reloading $name\"\n\tstart-stop-daemon --signal HUP --pidfile \"$pidfile\"\n\teend $?\n}\n",
    );
    Ok(script)
}

fn openrc_invocation(
    program: &Path,
    operation: &str,
    args: Vec<String>,
    service: &str,
) -> ProcessInvocation {
    ProcessInvocation {
        operation: operation.to_string(),
        program: program.to_path_buf(),
        args,
        unit: Some(service.to_string()),
        inherit_stdio: false,
    }
}

fn rc_service_invocation(rc_service: &Path, service: &str, action: &str) -> ProcessInvocation {
    openrc_invocation(
        rc_service,
        &format!("rc-service {action}"),
        vec![service.to_string(), action.to_string()],
        service,
    )
}

fn rc_update_invocation(rc_update: &Path, action: &str, service: &str) -> ProcessInvocation {
    openrc_invocation(
        rc_update,
        &format!("rc-update {action}"),
        vec![
            action.to_string(),
            service.to_string(),
            OPENRC_DEFAULT_RUNLEVEL.to_string(),
        ],
        service,
    )
}

fn rc_update_show_invocation(rc_update: &Path, service: &str) -> ProcessInvocation {
    openrc_invocation(
        rc_update,
        "rc-update show",
        vec!["show".to_string(), OPENRC_DEFAULT_RUNLEVEL.to_string()],
        service,
    )
}

fn openrc_service_active<E: ProcessExecutor>(
    executor: &mut E,
    rc_service: &Path,
    service: &str,
) -> bool {
    executor
        .execute(&rc_service_invocation(rc_service, service, "status"))
        .map(|output| output.success)
        .unwrap_or(false)
}

fn openrc_service_enabled(runlevel_dir: &Path, service: &str) -> bool {
    runlevel_dir
        .join(OPENRC_DEFAULT_RUNLEVEL)
        .join(service)
        .symlink_metadata()
        .is_ok()
}

/// `rc-update show <runlevel>` lists one `<service> | <runlevels>` row per
/// enabled service; match the exact first field.
fn runlevel_lists_service(show_output: &str, service: &str) -> bool {
    show_output
        .lines()
        .any(|line| line.split('|').next().map(str::trim) == Some(service))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpenrcInstallOutcome {
    pub(crate) service: String,
    pub(crate) enabled: bool,
    pub(crate) started: bool,
}

#[derive(Debug, Clone)]
struct OpenrcInstallSnapshot {
    previous_content: Option<String>,
    enabled: bool,
    active: bool,
}

fn execute_openrc_install_plan<E: ProcessExecutor>(
    executor: &mut E,
    rc_service: &Path,
    rc_update: &Path,
    service: &str,
    snapshot: &OpenrcInstallSnapshot,
    no_start: bool,
) -> Result<(), String> {
    if !snapshot.enabled {
        execute_required(executor, &rc_update_invocation(rc_update, "add", service))?;
    }
    if no_start {
        let output = execute_required(executor, &rc_update_show_invocation(rc_update, service))?;
        if !runlevel_lists_service(&output.stdout, service) {
            return Err(format!(
                "rc-update add reported success but {service} is not listed in the default runlevel"
            ));
        }
        return Ok(());
    }
    // An already-running service must adopt the newly written init script.
    let action = if snapshot.active { "restart" } else { "start" };
    execute_required(
        executor,
        &rc_service_invocation(rc_service, service, action),
    )?;
    execute_required(
        executor,
        &rc_service_invocation(rc_service, service, "status"),
    )?;
    Ok(())
}

fn rollback_openrc_install<E: ProcessExecutor>(
    executor: &mut E,
    rc_service: &Path,
    rc_update: &Path,
    init_file: &Path,
    service: &str,
    snapshot: &OpenrcInstallSnapshot,
    attempted_start: bool,
) -> Vec<String> {
    let mut errors = Vec::new();
    if attempted_start {
        best_effort_execute(
            executor,
            &rc_service_invocation(rc_service, service, "stop"),
            "stop failed",
            &mut errors,
        );
    }
    if !snapshot.enabled {
        best_effort_execute(
            executor,
            &rc_update_invocation(rc_update, "del", service),
            "rc-update del failed",
            &mut errors,
        );
    }
    if let Err(error) = restore_unit_file(init_file, snapshot.previous_content.as_deref()) {
        push_rollback_error(&mut errors, "init script restore failed", error);
    }
    if snapshot.active {
        best_effort_execute(
            executor,
            &rc_service_invocation(rc_service, service, "start"),
            "active state restore failed",
            &mut errors,
        );
    }
    errors
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn install_openrc_service_with_executor<E: ProcessExecutor>(
    executor: &mut E,
    rc_service: &Path,
    rc_update: &Path,
    init_file: &Path,
    runlevel_dir: &Path,
    service: &str,
    content: &str,
    overwrite: bool,
    no_start: bool,
) -> Result<OpenrcInstallOutcome, String> {
    validate_openrc_identity("service name", service)?;
    let existing_kind = preflight_service_path(init_file, overwrite, "OpenRC service")?;
    let snapshot = OpenrcInstallSnapshot {
        previous_content: match existing_kind {
            ExistingUnitKind::ManagedRegularFile => Some(
                std::fs::read_to_string(init_file)
                    .map_err(|e| format!("failed to read {}: {}", init_file.display(), e))?,
            ),
            ExistingUnitKind::Absent => None,
        },
        enabled: openrc_service_enabled(runlevel_dir, service),
        // Only probe a service that actually has an init script; rc-service
        // errors on unknown services, which is expected for a fresh install.
        active: match existing_kind {
            ExistingUnitKind::ManagedRegularFile => {
                openrc_service_active(executor, rc_service, service)
            }
            ExistingUnitKind::Absent => false,
        },
    };
    write_text_file_atomic_mode(init_file, content, overwrite, 0o755)?;
    if let Err(error) = execute_openrc_install_plan(
        executor, rc_service, rc_update, service, &snapshot, no_start,
    ) {
        let rollback_errors = rollback_openrc_install(
            executor, rc_service, rc_update, init_file, service, &snapshot, !no_start,
        );
        return Err(install_error_with_rollback(
            &format!("OpenRC service {service}"),
            error,
            rollback_errors,
        ));
    }
    Ok(OpenrcInstallOutcome {
        service: service.to_string(),
        enabled: true,
        started: !no_start,
    })
}

pub(crate) fn install_openrc_service(
    init_file: &Path,
    service: &str,
    content: &str,
    overwrite: bool,
    no_start: bool,
) -> Result<OpenrcInstallOutcome, String> {
    if init_file.parent() != Some(Path::new(OPENRC_INIT_DIR)) {
        return Err(format!(
            "OpenRC init scripts must live in {OPENRC_INIT_DIR}: {}",
            init_file.display()
        ));
    }
    let rc_service = rc_service_path()?;
    let rc_update = rc_update_path()?;
    let mut executor = RealProcessExecutor;
    install_openrc_service_with_executor(
        &mut executor,
        &rc_service,
        &rc_update,
        init_file,
        Path::new(OPENRC_RUNLEVELS_DIR),
        service,
        content,
        overwrite,
        no_start,
    )
}

pub(crate) fn control_openrc_service_with_executor<E: ProcessExecutor>(
    executor: &mut E,
    rc_service: &Path,
    service: &str,
    control: ServiceControl,
) -> Result<(), String> {
    validate_openrc_identity("service name", service)?;
    execute_required(
        executor,
        &rc_service_invocation(rc_service, service, control.as_str()),
    )?;
    if matches!(control, ServiceControl::Start | ServiceControl::Restart) {
        execute_required(
            executor,
            &rc_service_invocation(rc_service, service, "status"),
        )?;
    }
    Ok(())
}

pub(crate) fn control_openrc_service(service: &str, control: ServiceControl) -> Result<(), String> {
    let rc_service = rc_service_path()?;
    let mut executor = RealProcessExecutor;
    control_openrc_service_with_executor(&mut executor, &rc_service, service, control)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpenrcUninstallOutcome {
    pub(crate) service: String,
    pub(crate) removed: bool,
}

fn uninstall_openrc_error(service: &str, error: String, rollback_errors: Vec<String>) -> String {
    let mut message = format!("uninstallation failed for OpenRC service {service}: {error}");
    if !rollback_errors.is_empty() {
        let mut summary = rollback_errors.join("; ");
        if summary.len() > 600 {
            summary.truncate(600);
            summary.push_str("...");
        }
        message.push_str("; rollback also encountered: ");
        message.push_str(&summary);
    }
    message
}

pub(crate) fn uninstall_openrc_service_with_executor<E: ProcessExecutor>(
    executor: &mut E,
    rc_service: &Path,
    rc_update: &Path,
    init_file: &Path,
    runlevel_dir: &Path,
    service: &str,
) -> Result<OpenrcUninstallOutcome, String> {
    validate_openrc_identity("service name", service)?;
    let metadata = match std::fs::symlink_metadata(init_file) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "failed to inspect {}: {}",
                init_file.display(),
                error
            ));
        }
    };
    let Some(metadata) = metadata else {
        return Ok(OpenrcUninstallOutcome {
            service: service.to_string(),
            removed: false,
        });
    };
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "refusing to uninstall an OpenRC service through a symlinked init script: {}; remove the symlink explicitly before retrying",
            init_file.display()
        ));
    }
    if !metadata.is_file() {
        return Err(format!(
            "refusing to uninstall a non-regular OpenRC init script: {}",
            init_file.display()
        ));
    }
    let enabled = openrc_service_enabled(runlevel_dir, service);
    let active = openrc_service_active(executor, rc_service, service);
    if active {
        execute_required(
            executor,
            &rc_service_invocation(rc_service, service, "stop"),
        )?;
    }
    if enabled {
        if let Err(error) =
            execute_required(executor, &rc_update_invocation(rc_update, "del", service))
        {
            let mut rollback_errors = Vec::new();
            if active {
                best_effort_execute(
                    executor,
                    &rc_service_invocation(rc_service, service, "start"),
                    "active state restore failed",
                    &mut rollback_errors,
                );
            }
            return Err(uninstall_openrc_error(service, error, rollback_errors));
        }
    }
    if let Err(error) = std::fs::remove_file(init_file) {
        let mut rollback_errors = Vec::new();
        if enabled {
            best_effort_execute(
                executor,
                &rc_update_invocation(rc_update, "add", service),
                "enabled state restore failed",
                &mut rollback_errors,
            );
        }
        if active {
            best_effort_execute(
                executor,
                &rc_service_invocation(rc_service, service, "start"),
                "active state restore failed",
                &mut rollback_errors,
            );
        }
        return Err(uninstall_openrc_error(
            service,
            format!("failed to remove {}: {}", init_file.display(), error),
            rollback_errors,
        ));
    }
    Ok(OpenrcUninstallOutcome {
        service: service.to_string(),
        removed: true,
    })
}

pub(crate) fn uninstall_openrc_service(
    init_file: &Path,
    service: &str,
) -> Result<OpenrcUninstallOutcome, String> {
    let rc_service = rc_service_path()?;
    let rc_update = rc_update_path()?;
    let mut executor = RealProcessExecutor;
    uninstall_openrc_service_with_executor(
        &mut executor,
        &rc_service,
        &rc_update,
        init_file,
        Path::new(OPENRC_RUNLEVELS_DIR),
        service,
    )
}

/// OpenRC has no journald; Runner logs are the plain file supervise-daemon
/// writes. `--since` relies on journald timestamps and is rejected explicitly
/// instead of being silently ignored.
pub(crate) fn run_openrc_logs_with_executor<E: ProcessExecutor>(
    executor: &mut E,
    tail: &Path,
    log_file: &Path,
    service: &str,
    lines: u32,
    since: Option<&str>,
    follow: bool,
) -> Result<String, String> {
    if since.is_some() {
        return Err(
            "--since is not supported for OpenRC services: logs are plain files without journald timestamps; use --lines or --follow"
                .to_string(),
        );
    }
    if !log_file.is_file() {
        return Err(format!(
            "OpenRC log file {} does not exist yet; start the service first (supervise-daemon creates it)",
            log_file.display()
        ));
    }
    let log_path = log_file
        .to_str()
        .ok_or_else(|| format!("log path is not valid UTF-8: {}", log_file.display()))?;
    let mut args = vec!["-n".to_string(), lines.to_string()];
    if follow {
        args.push("-f".to_string());
    }
    args.push(log_path.to_string());
    let invocation = ProcessInvocation {
        operation: format!("tail {service} log"),
        program: tail.to_path_buf(),
        args,
        unit: Some(service.to_string()),
        inherit_stdio: follow,
    };
    let output = execute_required(executor, &invocation)?;
    Ok(output.stdout)
}

pub(crate) fn run_openrc_logs(
    service: &str,
    lines: u32,
    since: Option<&str>,
    follow: bool,
) -> Result<String, String> {
    let tail = tail_path()?;
    let log_file = openrc_log_file(service);
    let mut executor = RealProcessExecutor;
    run_openrc_logs_with_executor(
        &mut executor,
        &tail,
        &log_file,
        service,
        lines,
        since,
        follow,
    )
}

fn parse_openrc_status(stdout: &str) -> String {
    for line in stdout.lines() {
        if let Some(state) = line.trim().strip_prefix("* status:") {
            return match state.trim() {
                "started" => "active",
                "stopped" | "inactive" => "inactive",
                "crashed" => "failed",
                "starting" => "activating",
                "stopping" => "deactivating",
                _ => "unknown",
            }
            .to_string();
        }
    }
    "unknown".to_string()
}

/// Status projection matching the systemd backend's `SystemdStatus` shape so
/// `webcodex runner status` renders identically for both service managers.
pub(crate) fn query_openrc_service_status_with_executor<E: ProcessExecutor>(
    executor: &mut E,
    rc_service: &Path,
    init_file: &Path,
    runlevel_dir: &Path,
    service: &str,
) -> SystemdStatus {
    let loaded = match std::fs::symlink_metadata(init_file) {
        Ok(metadata) if metadata.is_file() || metadata.file_type().is_symlink() => "loaded",
        Ok(_) => "unknown",
        Err(_) => "not-found",
    };
    let active = match executor.execute(&rc_service_invocation(rc_service, service, "status")) {
        Ok(output) => parse_openrc_status(&output.stdout),
        Err(_) => "unknown".to_string(),
    };
    let enabled = if openrc_service_enabled(runlevel_dir, service) {
        "enabled"
    } else {
        "disabled"
    };
    SystemdStatus {
        loaded: loaded.to_string(),
        active,
        enabled: enabled.to_string(),
    }
}

pub(crate) fn query_openrc_service_status(init_file: &Path, service: &str) -> SystemdStatus {
    let Ok(rc_service) = rc_service_path() else {
        return SystemdStatus {
            loaded: "unknown".to_string(),
            active: "unknown".to_string(),
            enabled: "unknown".to_string(),
        };
    };
    let mut executor = RealProcessExecutor;
    query_openrc_service_status_with_executor(
        &mut executor,
        &rc_service,
        init_file,
        Path::new(OPENRC_RUNLEVELS_DIR),
        service,
    )
}

#[cfg(test)]
mod tests {
    use super::super::service::ProcessOutput;
    use super::*;

    #[derive(Default)]
    struct FakeExecutor {
        outputs: std::collections::VecDeque<Result<ProcessOutput, String>>,
        calls: Vec<Vec<String>>,
    }

    impl FakeExecutor {
        fn with_outputs(outputs: Vec<Result<ProcessOutput, String>>) -> Self {
            Self {
                outputs: outputs.into(),
                calls: Vec::new(),
            }
        }
    }

    impl ProcessExecutor for FakeExecutor {
        fn execute(&mut self, invocation: &ProcessInvocation) -> Result<ProcessOutput, String> {
            self.calls.push(invocation.args.clone());
            self.outputs
                .pop_front()
                .unwrap_or_else(|| panic!("missing fake output for {:?}", invocation.args))
        }
    }

    fn output(success: bool, stdout: &str, stderr: &str) -> Result<ProcessOutput, String> {
        Ok(ProcessOutput {
            success,
            code: Some(if success { 0 } else { 1 }),
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        })
    }

    fn ok() -> Result<ProcessOutput, String> {
        output(true, "", "")
    }

    fn failed(message: &str) -> Result<ProcessOutput, String> {
        output(false, "", message)
    }

    fn install_opts() -> RunnerInstallServiceOptions {
        RunnerInstallServiceOptions {
            scope: ServiceScope::System,
            service_manager: ServiceManagerSelection::Auto,
            config: PathBuf::from("/etc/webcodex/runner.toml"),
            bin: PathBuf::from("/usr/local/bin/webcodex-runner"),
            service_file: PathBuf::from(OPENRC_RUNNER_INIT_FILE),
            service_file_explicit: false,
            user: Some("webcodex".to_string()),
            group: Some("webcodex".to_string()),
            working_directory: PathBuf::from("/home/webcodex"),
            root_runner: false,
            allow_root_runner: false,
            overwrite: false,
            dry_run: false,
            output_stdout: false,
            no_start: false,
            json: false,
        }
    }

    fn render(opts: &RunnerInstallServiceOptions) -> String {
        render_runner_openrc_init(opts, Path::new(OPENRC_RUNNER_INIT_FILE))
            .expect("render should succeed")
    }

    #[test]
    fn render_uses_openrc_run_supervise_daemon_and_embeds_no_tokens() {
        let script = render(&install_opts());
        assert!(script.starts_with("#!/sbin/openrc-run\n"), "{script}");
        assert!(
            script.contains("supervisor=\"supervise-daemon\""),
            "{script}"
        );
        assert!(
            script.contains("command=\"${WEBCODEX_RUNNER_BIN:-/usr/local/bin/webcodex-runner}\""),
            "{script}"
        );
        assert!(
            script.contains(
                "command_args=\"--config ${WEBCODEX_RUNNER_CONFIG:-/etc/webcodex/runner.toml}\""
            ),
            "{script}"
        );
        assert!(
            script.contains("command_user=\"${WEBCODEX_RUNNER_USER:-webcodex:webcodex}\""),
            "{script}"
        );
        assert!(
            script.contains("directory=\"${WEBCODEX_RUNNER_WORKING_DIRECTORY:-/home/webcodex}\""),
            "{script}"
        );
        assert!(
            script.contains("pidfile=\"/run/${RC_SVCNAME}.pid\""),
            "{script}"
        );
        assert!(
            script.contains("output_log=\"${WEBCODEX_RUNNER_LOG:-/var/log/webcodex-runner.log}\""),
            "{script}"
        );
        assert!(
            script.contains("error_log=\"${WEBCODEX_RUNNER_LOG:-/var/log/webcodex-runner.log}\""),
            "{script}"
        );
        assert!(
            script.contains("extra_started_commands=\"reload\""),
            "{script}"
        );
        assert!(
            script.contains("start-stop-daemon --signal HUP --pidfile \"$pidfile\""),
            "{script}"
        );
        assert!(script.contains("depend()"), "{script}");
        // No secrets: the script references the config path only.
        assert!(!script.contains("token"), "{script}");
        assert!(!script.contains("systemctl"), "{script}");
        assert!(!script.contains("journalctl"), "{script}");
        assert!(!script.contains("bash"), "{script}");
    }

    #[test]
    fn render_rejects_root_runner_without_explicit_opt_in() {
        let mut opts = install_opts();
        opts.user = None;
        opts.group = None;
        opts.root_runner = true;
        let error = render_runner_openrc_init(&opts, Path::new(OPENRC_RUNNER_INIT_FILE))
            .expect_err("root runner without --allow-root-runner must fail");
        assert!(error.contains("--allow-root-runner"), "{error}");

        opts.allow_root_runner = true;
        let script = render(&opts);
        assert!(
            script.contains("--allow-root-runner was explicitly accepted"),
            "{script}"
        );
        assert!(!script.contains("command_user"), "{script}");
    }

    #[test]
    fn render_rejects_user_scope() {
        let mut opts = install_opts();
        opts.scope = ServiceScope::User;
        opts.user = None;
        opts.group = None;
        let error = render_runner_openrc_init(&opts, Path::new(OPENRC_RUNNER_INIT_FILE))
            .expect_err("user scope must fail for OpenRC");
        assert!(error.contains("--scope system"), "{error}");
    }

    #[test]
    fn render_rejects_group_without_user_and_unsafe_values() {
        let mut opts = install_opts();
        opts.user = None;
        let error = render_runner_openrc_init(&opts, Path::new(OPENRC_RUNNER_INIT_FILE))
            .expect_err("group without user must fail");
        assert!(error.contains("--group requires --user"), "{error}");

        let mut opts = install_opts();
        opts.config = PathBuf::from("/etc/webcodex/run ner.toml");
        let error = render_runner_openrc_init(&opts, Path::new(OPENRC_RUNNER_INIT_FILE))
            .expect_err("whitespace in the config path must fail");
        assert!(error.contains("whitespace"), "{error}");

        let mut opts = install_opts();
        opts.config = PathBuf::from("/etc/webcodex/ru\"nner.toml");
        let error = render_runner_openrc_init(&opts, Path::new(OPENRC_RUNNER_INIT_FILE))
            .expect_err("a double quote in the config path must fail");
        assert!(error.contains("unsafe"), "{error}");

        let mut opts = install_opts();
        opts.user = Some("webcodex;id".to_string());
        let error = render_runner_openrc_init(&opts, Path::new(OPENRC_RUNNER_INIT_FILE))
            .expect_err("shell metacharacters in the user must fail");
        assert!(error.contains("invalid OpenRC user value"), "{error}");

        let mut opts = install_opts();
        opts.bin = PathBuf::from("relative/webcodex-runner");
        let error = render_runner_openrc_init(&opts, Path::new(OPENRC_RUNNER_INIT_FILE))
            .expect_err("a relative binary path must fail");
        assert!(error.contains("must be absolute"), "{error}");
    }

    #[test]
    fn detect_service_manager_matrix() {
        let probe = |openrc_runtime, systemd_runtime, rc_service, rc_update, systemctl| {
            ServiceManagerProbe {
                openrc_runtime,
                systemd_runtime,
                rc_service,
                rc_update,
                systemctl,
            }
        };
        let auto = ServiceManagerSelection::Auto;
        // Runtime directories win.
        assert_eq!(
            select_service_manager(auto, &probe(true, false, true, true, false)).unwrap(),
            ServiceManager::Openrc
        );
        assert_eq!(
            select_service_manager(auto, &probe(false, true, false, false, true)).unwrap(),
            ServiceManager::Systemd
        );
        // Both runtimes or no runtime with both toolsets are ambiguous.
        assert!(select_service_manager(auto, &probe(true, true, true, true, true)).is_err());
        assert!(select_service_manager(auto, &probe(false, false, true, true, true)).is_err());
        // No runtime directories: fall back to the single available toolset.
        assert_eq!(
            select_service_manager(auto, &probe(false, false, false, false, true)).unwrap(),
            ServiceManager::Systemd
        );
        assert_eq!(
            select_service_manager(auto, &probe(false, false, true, true, false)).unwrap(),
            ServiceManager::Openrc
        );
        // OpenRC runtime without its tools, or nothing at all, fails closed.
        let error = select_service_manager(auto, &probe(true, false, false, true, false))
            .expect_err("OpenRC runtime without rc-service/rc-update must fail");
        assert!(error.contains("rc-service"), "{error}");
        assert!(select_service_manager(auto, &probe(false, false, false, false, false)).is_err());
        // Explicit selections never probe.
        assert_eq!(
            select_service_manager(
                ServiceManagerSelection::Openrc,
                &probe(false, true, false, false, true)
            )
            .unwrap(),
            ServiceManager::Openrc
        );
        assert_eq!(
            select_service_manager(
                ServiceManagerSelection::Systemd,
                &probe(true, false, true, true, false)
            )
            .unwrap(),
            ServiceManager::Systemd
        );
    }

    #[test]
    fn service_manager_selection_parse() {
        assert_eq!(
            ServiceManagerSelection::parse("auto").unwrap(),
            ServiceManagerSelection::Auto
        );
        assert_eq!(
            ServiceManagerSelection::parse("systemd").unwrap(),
            ServiceManagerSelection::Systemd
        );
        assert_eq!(
            ServiceManagerSelection::parse("openrc").unwrap(),
            ServiceManagerSelection::Openrc
        );
        assert!(ServiceManagerSelection::parse("runit").is_err());
    }

    #[test]
    fn control_maps_to_rc_service_commands() {
        let rc_service = Path::new("/sbin/rc-service");

        let mut start = FakeExecutor::with_outputs(vec![ok(), ok()]);
        control_openrc_service_with_executor(
            &mut start,
            rc_service,
            "webcodex-runner",
            ServiceControl::Start,
        )
        .unwrap();
        assert_eq!(
            start.calls,
            [
                ["webcodex-runner".to_string(), "start".to_string()],
                ["webcodex-runner".to_string(), "status".to_string()]
            ]
        );

        let mut stop = FakeExecutor::with_outputs(vec![ok()]);
        control_openrc_service_with_executor(
            &mut stop,
            rc_service,
            "webcodex-runner",
            ServiceControl::Stop,
        )
        .unwrap();
        assert_eq!(
            stop.calls,
            [["webcodex-runner".to_string(), "stop".to_string()]]
        );

        let mut restart = FakeExecutor::with_outputs(vec![ok(), ok()]);
        control_openrc_service_with_executor(
            &mut restart,
            rc_service,
            "webcodex-runner",
            ServiceControl::Restart,
        )
        .unwrap();
        assert_eq!(
            restart.calls,
            [
                ["webcodex-runner".to_string(), "restart".to_string()],
                ["webcodex-runner".to_string(), "status".to_string()]
            ]
        );

        let mut reload = FakeExecutor::with_outputs(vec![ok()]);
        control_openrc_service_with_executor(
            &mut reload,
            rc_service,
            "webcodex-runner",
            ServiceControl::Reload,
        )
        .unwrap();
        assert_eq!(
            reload.calls,
            [["webcodex-runner".to_string(), "reload".to_string()]]
        );
    }

    #[test]
    fn install_no_start_enables_default_runlevel_without_starting() {
        let tmp = tempfile::tempdir().unwrap();
        let init_file = tmp.path().join("webcodex-runner");
        let runlevel_dir = tmp.path().join("runlevels");
        let mut executor = FakeExecutor::with_outputs(vec![
            ok(),
            output(true, "  webcodex-runner |      default\n", ""),
        ]);
        let outcome = install_openrc_service_with_executor(
            &mut executor,
            Path::new("/sbin/rc-service"),
            Path::new("/sbin/rc-update"),
            &init_file,
            &runlevel_dir,
            "webcodex-runner",
            "#!/sbin/openrc-run\n",
            false,
            true,
        )
        .unwrap();
        assert!(!outcome.started);
        assert!(outcome.enabled);
        assert_eq!(outcome.service, "webcodex-runner");
        assert_eq!(executor.calls.len(), 2);
        assert_eq!(executor.calls[0], ["add", "webcodex-runner", "default"]);
        assert_eq!(executor.calls[1], ["show", "default"]);
        assert_eq!(
            std::fs::read_to_string(&init_file).unwrap(),
            "#!/sbin/openrc-run\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&init_file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755, "init scripts must be executable");
        }
    }

    #[test]
    fn install_start_uses_rc_service_and_verifies_status() {
        let tmp = tempfile::tempdir().unwrap();
        let init_file = tmp.path().join("webcodex-runner");
        let runlevel_dir = tmp.path().join("runlevels");
        let mut executor = FakeExecutor::with_outputs(vec![ok(), ok(), ok()]);
        let outcome = install_openrc_service_with_executor(
            &mut executor,
            Path::new("/sbin/rc-service"),
            Path::new("/sbin/rc-update"),
            &init_file,
            &runlevel_dir,
            "webcodex-runner",
            "#!/sbin/openrc-run\n",
            false,
            false,
        )
        .unwrap();
        assert!(outcome.started);
        assert_eq!(executor.calls.len(), 3);
        assert_eq!(executor.calls[0], ["add", "webcodex-runner", "default"]);
        assert_eq!(executor.calls[1], ["webcodex-runner", "start"]);
        assert_eq!(executor.calls[2], ["webcodex-runner", "status"]);
    }

    #[test]
    fn install_restart_adopts_new_script_when_service_was_active() {
        let tmp = tempfile::tempdir().unwrap();
        let init_file = tmp.path().join("webcodex-runner");
        std::fs::write(&init_file, "old script").unwrap();
        let runlevel_dir = tmp.path().join("runlevels");
        std::fs::create_dir_all(runlevel_dir.join("default")).unwrap();
        std::fs::write(runlevel_dir.join("default").join("webcodex-runner"), "").unwrap();
        // Snapshot observes the service active, then restart + status verify.
        let mut executor = FakeExecutor::with_outputs(vec![ok(), ok(), ok()]);
        let outcome = install_openrc_service_with_executor(
            &mut executor,
            Path::new("/sbin/rc-service"),
            Path::new("/sbin/rc-update"),
            &init_file,
            &runlevel_dir,
            "webcodex-runner",
            "#!/sbin/openrc-run\n# new\n",
            true,
            false,
        )
        .unwrap();
        assert!(outcome.started);
        assert_eq!(
            executor.calls,
            [
                ["webcodex-runner".to_string(), "status".to_string()],
                ["webcodex-runner".to_string(), "restart".to_string()],
                ["webcodex-runner".to_string(), "status".to_string()]
            ]
        );
        assert_eq!(
            std::fs::read_to_string(&init_file).unwrap(),
            "#!/sbin/openrc-run\n# new\n"
        );
    }

    #[test]
    fn install_refuses_overwrite_without_flag() {
        let tmp = tempfile::tempdir().unwrap();
        let init_file = tmp.path().join("webcodex-runner");
        std::fs::write(&init_file, "old script").unwrap();
        let runlevel_dir = tmp.path().join("runlevels");
        let mut executor = FakeExecutor::with_outputs(vec![]);
        let error = install_openrc_service_with_executor(
            &mut executor,
            Path::new("/sbin/rc-service"),
            Path::new("/sbin/rc-update"),
            &init_file,
            &runlevel_dir,
            "webcodex-runner",
            "#!/sbin/openrc-run\n",
            false,
            true,
        )
        .expect_err("overwrite without the flag must fail");
        assert!(
            error.contains("already exists; pass --overwrite"),
            "{error}"
        );
        assert_eq!(std::fs::read_to_string(&init_file).unwrap(), "old script");
        assert!(executor.calls.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn install_refuses_symlink_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        std::fs::write(&target, "target").unwrap();
        let init_file = tmp.path().join("webcodex-runner");
        std::os::unix::fs::symlink(&target, &init_file).unwrap();
        let runlevel_dir = tmp.path().join("runlevels");
        let mut executor = FakeExecutor::with_outputs(vec![]);
        let error = install_openrc_service_with_executor(
            &mut executor,
            Path::new("/sbin/rc-service"),
            Path::new("/sbin/rc-update"),
            &init_file,
            &runlevel_dir,
            "webcodex-runner",
            "#!/sbin/openrc-run\n",
            true,
            true,
        )
        .expect_err("symlink overwrite must fail");
        assert!(error.contains("OpenRC service symlink"), "{error}");
        assert!(executor.calls.is_empty());
    }

    #[test]
    fn install_failure_rolls_back_new_file_and_runlevel() {
        let tmp = tempfile::tempdir().unwrap();
        let init_file = tmp.path().join("webcodex-runner");
        let runlevel_dir = tmp.path().join("runlevels");
        let mut executor = FakeExecutor::with_outputs(vec![
            failed("rc-update add boom"),
            ok(), // rollback stop (tolerated)
            ok(), // rollback rc-update del
        ]);
        let error = install_openrc_service_with_executor(
            &mut executor,
            Path::new("/sbin/rc-service"),
            Path::new("/sbin/rc-update"),
            &init_file,
            &runlevel_dir,
            "webcodex-runner",
            "#!/sbin/openrc-run\n",
            false,
            false,
        )
        .expect_err("install must fail");
        assert!(
            error.contains("installation failed for OpenRC service webcodex-runner"),
            "{error}"
        );
        assert!(error.contains("rc-update add boom"), "{error}");
        // Newly written init script is removed again by the rollback.
        assert!(!init_file.exists());
        assert_eq!(executor.calls.len(), 3);
        assert_eq!(executor.calls[0], ["add", "webcodex-runner", "default"]);
        assert_eq!(executor.calls[1], ["webcodex-runner", "stop"]);
        assert_eq!(executor.calls[2], ["del", "webcodex-runner", "default"]);
    }

    #[test]
    fn install_failure_restores_previous_file_and_active_state() {
        let tmp = tempfile::tempdir().unwrap();
        let init_file = tmp.path().join("webcodex-runner");
        std::fs::write(&init_file, "old script").unwrap();
        let runlevel_dir = tmp.path().join("runlevels");
        std::fs::create_dir_all(runlevel_dir.join("default")).unwrap();
        std::fs::write(runlevel_dir.join("default").join("webcodex-runner"), "").unwrap();
        let mut executor = FakeExecutor::with_outputs(vec![
            ok(),                   // snapshot status probe: active
            failed("restart boom"), // plan restart fails
            ok(),                   // rollback stop
            ok(),                   // rollback start (restore active state)
        ]);
        let error = install_openrc_service_with_executor(
            &mut executor,
            Path::new("/sbin/rc-service"),
            Path::new("/sbin/rc-update"),
            &init_file,
            &runlevel_dir,
            "webcodex-runner",
            "#!/sbin/openrc-run\n# new\n",
            true,
            false,
        )
        .expect_err("install must fail");
        assert!(error.contains("restart boom"), "{error}");
        // The previously installed script content is restored.
        assert_eq!(std::fs::read_to_string(&init_file).unwrap(), "old script");
        assert_eq!(
            executor.calls,
            [
                ["webcodex-runner".to_string(), "status".to_string()],
                ["webcodex-runner".to_string(), "restart".to_string()],
                ["webcodex-runner".to_string(), "stop".to_string()],
                ["webcodex-runner".to_string(), "start".to_string()]
            ]
        );
    }

    #[test]
    fn install_no_start_runlevel_verification_failure_rolls_back() {
        let tmp = tempfile::tempdir().unwrap();
        let init_file = tmp.path().join("webcodex-runner");
        let runlevel_dir = tmp.path().join("runlevels");
        let mut executor = FakeExecutor::with_outputs(vec![
            ok(),
            output(true, "  other-service | default\n", ""),
            ok(), // rollback rc-update del
        ]);
        let error = install_openrc_service_with_executor(
            &mut executor,
            Path::new("/sbin/rc-service"),
            Path::new("/sbin/rc-update"),
            &init_file,
            &runlevel_dir,
            "webcodex-runner",
            "#!/sbin/openrc-run\n",
            false,
            true,
        )
        .expect_err("install must fail when the service is not listed");
        assert!(
            error.contains("not listed in the default runlevel"),
            "{error}"
        );
        assert!(!init_file.exists());
        // No start attempt in --no-start mode: no rc-service calls at all.
        assert!(executor
            .calls
            .iter()
            .all(|call| call.first().map(String::as_str) != Some("webcodex-runner")));
    }

    #[test]
    fn uninstall_stops_disables_and_removes_the_init_script() {
        let tmp = tempfile::tempdir().unwrap();
        let init_file = tmp.path().join("webcodex-runner");
        std::fs::write(&init_file, "#!/sbin/openrc-run\n").unwrap();
        let runlevel_dir = tmp.path().join("runlevels");
        std::fs::create_dir_all(runlevel_dir.join("default")).unwrap();
        std::fs::write(runlevel_dir.join("default").join("webcodex-runner"), "").unwrap();
        let mut executor = FakeExecutor::with_outputs(vec![
            ok(), // status probe: active
            ok(), // stop
            ok(), // rc-update del
        ]);
        let outcome = uninstall_openrc_service_with_executor(
            &mut executor,
            Path::new("/sbin/rc-service"),
            Path::new("/sbin/rc-update"),
            &init_file,
            &runlevel_dir,
            "webcodex-runner",
        )
        .unwrap();
        assert!(outcome.removed);
        assert!(!init_file.exists());
        assert_eq!(executor.calls.len(), 3);
        assert_eq!(executor.calls[0], ["webcodex-runner", "status"]);
        assert_eq!(executor.calls[1], ["webcodex-runner", "stop"]);
        assert_eq!(executor.calls[2], ["del", "webcodex-runner", "default"]);
    }

    #[test]
    fn uninstall_absent_service_is_a_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let init_file = tmp.path().join("webcodex-runner");
        let runlevel_dir = tmp.path().join("runlevels");
        let mut executor = FakeExecutor::with_outputs(vec![]);
        let outcome = uninstall_openrc_service_with_executor(
            &mut executor,
            Path::new("/sbin/rc-service"),
            Path::new("/sbin/rc-update"),
            &init_file,
            &runlevel_dir,
            "webcodex-runner",
        )
        .unwrap();
        assert!(!outcome.removed);
        assert!(executor.calls.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn uninstall_refuses_a_symlinked_init_script() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        std::fs::write(&target, "#!/sbin/openrc-run\n").unwrap();
        let init_file = tmp.path().join("webcodex-runner");
        std::os::unix::fs::symlink(&target, &init_file).unwrap();
        let runlevel_dir = tmp.path().join("runlevels");
        let mut executor = FakeExecutor::with_outputs(vec![]);
        let error = uninstall_openrc_service_with_executor(
            &mut executor,
            Path::new("/sbin/rc-service"),
            Path::new("/sbin/rc-update"),
            &init_file,
            &runlevel_dir,
            "webcodex-runner",
        )
        .expect_err("symlinked init script must be refused");
        assert!(error.contains("symlink"), "{error}");
        assert!(init_file.symlink_metadata().is_ok());
        assert!(executor.calls.is_empty());
    }

    #[test]
    fn uninstall_disable_failure_restores_active_state() {
        let tmp = tempfile::tempdir().unwrap();
        let init_file = tmp.path().join("webcodex-runner");
        std::fs::write(&init_file, "#!/sbin/openrc-run\n").unwrap();
        let runlevel_dir = tmp.path().join("runlevels");
        std::fs::create_dir_all(runlevel_dir.join("default")).unwrap();
        std::fs::write(runlevel_dir.join("default").join("webcodex-runner"), "").unwrap();
        let mut executor = FakeExecutor::with_outputs(vec![
            ok(),                         // status probe: active
            ok(),                         // stop
            failed("rc-update del boom"), // disable fails
            ok(),                         // rollback start
        ]);
        let error = uninstall_openrc_service_with_executor(
            &mut executor,
            Path::new("/sbin/rc-service"),
            Path::new("/sbin/rc-update"),
            &init_file,
            &runlevel_dir,
            "webcodex-runner",
        )
        .expect_err("uninstall must fail");
        assert!(
            error.contains("uninstallation failed for OpenRC service webcodex-runner"),
            "{error}"
        );
        assert!(error.contains("rc-update del boom"), "{error}");
        // The init script stays in place and the service is started again.
        assert!(init_file.exists());
        assert_eq!(executor.calls.len(), 4);
        assert_eq!(executor.calls[0], ["webcodex-runner", "status"]);
        assert_eq!(executor.calls[1], ["webcodex-runner", "stop"]);
        assert_eq!(executor.calls[2], ["del", "webcodex-runner", "default"]);
        assert_eq!(executor.calls[3], ["webcodex-runner", "start"]);
    }

    #[test]
    fn logs_reject_since_instead_of_ignoring_it() {
        let tmp = tempfile::tempdir().unwrap();
        let log_file = tmp.path().join("webcodex-runner.log");
        std::fs::write(&log_file, "line\n").unwrap();
        let mut executor = FakeExecutor::with_outputs(vec![]);
        let error = run_openrc_logs_with_executor(
            &mut executor,
            Path::new("/usr/bin/tail"),
            &log_file,
            "webcodex-runner",
            200,
            Some("1 hour ago"),
            false,
        )
        .expect_err("--since must fail explicitly for OpenRC");
        assert!(
            error.contains("--since is not supported for OpenRC"),
            "{error}"
        );
        assert!(executor.calls.is_empty());
    }

    #[test]
    fn logs_require_an_existing_log_file() {
        let tmp = tempfile::tempdir().unwrap();
        let log_file = tmp.path().join("missing.log");
        let mut executor = FakeExecutor::with_outputs(vec![]);
        let error = run_openrc_logs_with_executor(
            &mut executor,
            Path::new("/usr/bin/tail"),
            &log_file,
            "webcodex-runner",
            200,
            None,
            false,
        )
        .expect_err("a missing log file must fail");
        assert!(error.contains("does not exist yet"), "{error}");
        assert!(executor.calls.is_empty());
    }

    #[test]
    fn logs_map_to_tail_with_lines_and_follow() {
        let tmp = tempfile::tempdir().unwrap();
        let log_file = tmp.path().join("webcodex-runner.log");
        std::fs::write(&log_file, "line1\nline2\n").unwrap();
        let log_path = log_file.to_str().unwrap().to_string();

        let mut executor = FakeExecutor::with_outputs(vec![output(true, "line2\n", "")]);
        let text = run_openrc_logs_with_executor(
            &mut executor,
            Path::new("/usr/bin/tail"),
            &log_file,
            "webcodex-runner",
            50,
            None,
            false,
        )
        .unwrap();
        assert_eq!(text, "line2\n");
        assert_eq!(
            executor.calls,
            [["-n".to_string(), "50".to_string(), log_path.clone()]]
        );

        let mut follow = FakeExecutor::with_outputs(vec![ok()]);
        run_openrc_logs_with_executor(
            &mut follow,
            Path::new("/usr/bin/tail"),
            &log_file,
            "webcodex-runner",
            50,
            None,
            true,
        )
        .unwrap();
        assert_eq!(
            follow.calls,
            [[
                "-n".to_string(),
                "50".to_string(),
                "-f".to_string(),
                log_path
            ]]
        );
    }

    #[test]
    fn status_maps_openrc_output_to_the_shared_projection() {
        let tmp = tempfile::tempdir().unwrap();
        let init_file = tmp.path().join("webcodex-runner");
        std::fs::write(&init_file, "#!/sbin/openrc-run\n").unwrap();
        let runlevel_dir = tmp.path().join("runlevels");
        std::fs::create_dir_all(runlevel_dir.join("default")).unwrap();
        std::fs::write(runlevel_dir.join("default").join("webcodex-runner"), "").unwrap();

        let mut executor =
            FakeExecutor::with_outputs(vec![output(true, " * status: started\n", "")]);
        let status = query_openrc_service_status_with_executor(
            &mut executor,
            Path::new("/sbin/rc-service"),
            &init_file,
            &runlevel_dir,
            "webcodex-runner",
        );
        assert_eq!(status.loaded, "loaded");
        assert_eq!(status.active, "active");
        assert_eq!(status.enabled, "enabled");

        let mut stopped =
            FakeExecutor::with_outputs(vec![output(false, " * status: stopped\n", "")]);
        let status = query_openrc_service_status_with_executor(
            &mut stopped,
            Path::new("/sbin/rc-service"),
            &init_file,
            &runlevel_dir,
            "webcodex-runner",
        );
        assert_eq!(status.active, "inactive");

        let mut crashed =
            FakeExecutor::with_outputs(vec![output(false, " * status: crashed\n", "")]);
        let status = query_openrc_service_status_with_executor(
            &mut crashed,
            Path::new("/sbin/rc-service"),
            &init_file,
            &runlevel_dir,
            "webcodex-runner",
        );
        assert_eq!(status.active, "failed");

        // A service without an init script reports not-found and disabled.
        let missing = tmp.path().join("missing");
        let mut executor = FakeExecutor::with_outputs(vec![failed("does not exist")]);
        let status = query_openrc_service_status_with_executor(
            &mut executor,
            Path::new("/sbin/rc-service"),
            &missing,
            &runlevel_dir,
            "missing",
        );
        assert_eq!(status.loaded, "not-found");
        assert_eq!(status.active, "unknown");
        assert_eq!(status.enabled, "disabled");
    }

    #[test]
    fn runlevel_listing_matches_the_exact_service_name() {
        let show = "          bootmisc | boot\n  webcodex-runner |      default\n   webcodex-runner-extra | default\n";
        assert!(runlevel_lists_service(show, "webcodex-runner"));
        assert!(!runlevel_lists_service(show, "webcodex"));
        assert!(!runlevel_lists_service("", "webcodex-runner"));
    }

    #[test]
    fn openrc_service_name_uses_the_init_file_basename() {
        assert_eq!(
            openrc_service_name(Path::new("/etc/init.d/webcodex-runner")),
            "webcodex-runner"
        );
        assert_eq!(
            openrc_service_name(Path::new("/etc/init.d/custom")),
            "custom"
        );
        assert_eq!(openrc_service_name(Path::new("")), RUNNER_OPENRC_SERVICE);
    }
}
