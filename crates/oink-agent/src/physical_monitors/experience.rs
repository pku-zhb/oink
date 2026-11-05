//! Shared setup and asynchronous, reversible physical desktop operations.
pub use super::model::{DesktopPlan, Endpoint, Mapping, Mode, Panel};
use super::{
    desktop,
    hardware::{Platform, Real},
    recovery::{self, Record},
    transport,
};
use crate::{
    api::{DeviceKey, ScreenKey},
    node::{Node, lock},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Mutex, mpsc},
    time::Duration,
};

pub const CAP: &str = "oink.physical-monitors/2";
const CONFIRM_MS: u64 = 15_000;
const GUARD_MS: u64 = 60_000;
const COOLDOWN_MS: u64 = 5_000;
type Pending = BTreeMap<
    (DeviceKey, [u8; 16]),
    (
        oink_foundation::session::Boot,
        mpsc::Sender<Result<serde_json::Value, String>>,
    ),
>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Status,
    Confirmation,
    Identify {
        device: DeviceKey,
        display: String,
        label: String,
    },
    Inspect {
        device: DeviceKey,
    },
    Read {
        device: DeviceKey,
        display: String,
    },
    Save {
        expected_revision: u64,
        mapping: Mapping,
    },
    Calibrate {
        panel: Panel,
        target: DeviceKey,
    },
    Apply {
        expected_revision: u64,
        choices: Vec<ScreenKey>,
        mode: Mode,
        baseline_confirmed: bool,
    },
    Keep {
        operation: String,
    },
    Revert {
        operation: String,
    },
    Restore,
    RecoverLocal,
    RecoverCalibration {
        operation: String,
    },
    Recovered {
        operation: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum PeerRequest {
    Client {
        request: Box<Request>,
    },
    Inspect,
    Identify {
        display: String,
        label: String,
    },
    Read {
        display: String,
    },
    Install {
        expected_revision: u64,
        mapping: Mapping,
    },
    UndoInstall {
        revision: u64,
        previous: Option<Mapping>,
    },
    Publish {
        revision: u64,
        choices: Vec<ScreenKey>,
        mode: Mode,
    },
    Prepare {
        operation: String,
        inputs: Vec<recovery::Input>,
        mode: Mode,
        plan: DesktopPlan,
    },
    Connect {
        operation: String,
    },
    Desktop {
        operation: String,
        plan: DesktopPlan,
    },
    Await {
        operation: String,
        deadline_ms: u64,
    },
    Finish {
        operation: String,
    },
    Rollback {
        operation: String,
    },
    RestoreEntry,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Store {
    pub mapping: Option<Mapping>,
    #[serde(default)]
    pub active: Vec<ScreenKey>,
    pub mode: Option<Mode>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Operation {
    pub id: String,
    pub phase: String,
    pub detail: String,
    pub deadline_ms: Option<u64>,
    pub choices: Vec<ScreenKey>,
    pub mode: Option<Mode>,
    pub hardware_verified: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Confirmation {
    pub operation: String,
    pub phase: String,
    pub deadline_ms: u64,
    pub bounds: Option<oink_foundation::Rect>,
    pub previous_bounds: Option<oink_foundation::Rect>,
    pub macos: bool,
}
impl Operation {
    pub(super) fn busy(&self) -> bool {
        matches!(
            self.phase.as_str(),
            "checking"
                | "preparing"
                | "connecting"
                | "switching"
                | "desktop"
                | "awaiting_confirmation"
                | "restoring"
                | "calibrating"
        )
    }
}
#[derive(Default)]
pub struct Runtime {
    pub(super) hardware: std::sync::OnceLock<std::sync::Arc<dyn Platform>>,
    state: Mutex<Option<Store>>,
    pub(super) operation: Mutex<Option<Operation>>,
    local_gate: Mutex<()>,
    local: Mutex<Option<Record>>,
    decision: Mutex<Option<(String, mpsc::Sender<bool>)>>,
    pub(super) cooldown: Mutex<u64>,
    pub(super) pending: Mutex<Pending>,
    #[cfg(test)]
    pub(super) confirm_ms: std::sync::atomic::AtomicU64,
}
impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhysicalMonitorRuntime").finish_non_exhaustive()
    }
}
fn hardware(node: &Node) -> &dyn Platform {
    node.inner
        .physical
        .hardware
        .get_or_init(|| std::sync::Arc::new(Real))
        .as_ref()
}
fn restore_record(node: &Node, record: &Record) -> Result<(), String> {
    hardware(node).restore_record(record)
}
fn root(node: &Node) -> PathBuf {
    node.data_dir().join("physical-monitors")
}
pub(super) fn store(node: &Node) -> Result<Store, String> {
    let mut state = lock(&node.inner.physical.state);
    if state.is_none() {
        let path = root(node).join("mapping.json");
        let loaded = match std::fs::read(path) {
            Ok(bytes) if bytes.len() <= 32768 => {
                serde_json::from_slice::<Store>(&bytes).map_err(|_| "Physical monitor configuration is unreadable")?
            }
            Ok(_) => return Err("Physical monitor configuration is too large".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Store::default(),
            Err(e) => return Err(e.to_string()),
        };
        *state = Some(loaded);
    }
    Ok(state.as_ref().expect("initialized").clone())
}
fn persist(node: &Node, state: Store) -> Result<(), String> {
    std::fs::create_dir_all(root(node)).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(&state).map_err(|e| e.to_string())?;
    if bytes.len() > 32768 {
        return Err("Physical monitor configuration is too large".into());
    }
    oink_foundation::storage::atomic_write(&root(node).join("mapping.json"), &bytes).map_err(|e| e.to_string())?;
    *lock(&node.inner.physical.state) = Some(state);
    Ok(())
}
fn group(node: &Node) -> Result<String, String> {
    Ok(node
        .link_config()
        .map_err(|e| e.message().to_string())?
        .applied()
        .group
        .0
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
fn id() -> String {
    crate::api::request_context::new_client()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn entry(node: &Node) -> Result<Option<Record>, String> {
    recovery::read(&root(node).join("entry-recovery.json"))
}
fn phase(node: &Node, name: &str, detail: impl Into<String>, deadline: Option<u64>) {
    if let Some(operation) = lock(&node.inner.physical.operation).as_mut() {
        operation.phase = name.into();
        operation.detail = detail.into();
        operation.deadline_ms = deadline;
    }
}
fn busy(node: &Node) -> bool {
    lock(&node.inner.physical.operation)
        .as_ref()
        .is_some_and(Operation::busy)
}
pub(super) fn legacy_guard(node: &Node, command: &super::Command) -> Result<(), String> {
    if matches!(command, super::Command::Query) {
        return Ok(());
    }
    if busy(node) || recovery::read(&recovery::path(node.data_dir()))?.is_some_and(|r| r.outcome.is_none()) {
        return Err("Wait for the shared monitor operation or its recovery to finish".into());
    }
    if !matches!(command, super::Command::Read { .. }) && store(node)?.mapping.is_some() {
        return Err("Use the shared monitor workflow to restore inputs and system desktops together".into());
    }
    Ok(())
}
fn same_operation(node: &Node, operation: &str) -> Result<Record, String> {
    let record = recovery::read(&recovery::path(node.data_dir()))?.ok_or("This operation has already ended")?;
    if record.id != operation || record.outcome.is_some() {
        return Err("This operation has already ended".into());
    }
    if record.owner_boot != Some(node.boot().nonce) {
        return Err("The display owner restarted; restore the previous operation before continuing".into());
    }
    if recovery::now_ms() >= record.deadline_ms {
        return Err("The operation expired; recovery is in progress".into());
    }
    if node.native_services_enabled() && (!record.ready || !record.helper_pid.is_some_and(recovery::alive)) {
        return Err("The independent recovery helper is unavailable; restoring before another change".into());
    }
    Ok(record)
}
fn coordinator(node: &Node, request: &Request) -> Result<DeviceKey, String> {
    if let Some(mapping) = store(node)?.mapping {
        return Ok(mapping.coordinator);
    }
    if let Request::Save { mapping, .. } = request {
        return Ok(mapping.coordinator);
    }
    Ok(node.id().into())
}
pub(super) fn require_mapping(node: &Node) -> Result<Mapping, String> {
    let mapping = store(node)?
        .mapping
        .ok_or("Set up the shared physical monitors first")?;
    if mapping.group != group(node)? {
        return Err("This setup belongs to a different paired group".into());
    }
    mapping.validate()?;
    Ok(mapping)
}
fn participants(node: &Node, mapping: &Mapping) -> Result<Vec<DeviceKey>, String> {
    let config = node.link_config().map_err(|e| e.message().to_string())?;
    let members = config.members();
    let peers: Vec<_> = mapping.participants().into_iter().collect();
    for peer in &peers {
        if !members
            .members
            .iter()
            .any(|m| m.device == peer.0 && (m.is_self || m.online && config.peer_supports(peer.0, CAP)))
        {
            return Err("A participating computer is offline or needs the physical monitor update".into());
        }
    }
    Ok(peers)
}
fn migration(node: &Node) -> Option<Mapping> {
    let setup: super::Setup =
        serde_json::from_slice(&std::fs::read(node.data_dir().join("physical-monitors.json")).ok()?).ok()?;
    if setup.monitors.is_empty() {
        return None;
    }
    let layout = node.link_config().ok()?.applied().layout.clone();
    let panels: Option<Vec<_>> = setup
        .monitors
        .iter()
        .map(|m| {
            let control = layout
                .screens
                .iter()
                .find(|s| s.id.device == node.id() && s.id.display == m.controller_display)?;
            Some(Panel {
                id: id(),
                name: m.name.clone(),
                width: control.rect.w,
                height: control.rect.h,
                endpoints: m
                    .bindings
                    .iter()
                    .map(|b| Endpoint {
                        screen: b.screen.clone(),
                        input: b.input,
                        port: String::new(),
                    })
                    .collect(),
                readback: m.readback,
                baseline: m
                    .bindings
                    .iter()
                    .find(|b| Some(b.input) == m.initial_input)
                    .or(m.bindings.first())?
                    .screen
                    .device,
                calibrated: false,
            })
        })
        .collect();
    let panels = panels?;
    let coordinator = panels
        .iter()
        .flat_map(|p| p.endpoints.iter().map(|e| e.screen.device))
        .min()?;
    Some(Mapping {
        revision: 0,
        group: group(node).ok()?,
        coordinator,
        panels,
    })
}
fn status(node: &Node) -> Result<serde_json::Value, String> {
    let state = store(node)?;
    let record = recovery::read(&recovery::path(node.data_dir()))?;
    let mut operation = lock(&node.inner.physical.operation).clone();
    if let Some(record) = &record
        && operation.as_ref().is_none_or(|op| !op.busy() && op.id != record.id)
        && !matches!(record.outcome, Some(Ok(())))
    {
        operation=Some(Operation {id:record.id.clone(),phase:"needs_attention".into(),detail:record.outcome.as_ref().and_then(|r|r.as_ref().err()).cloned().unwrap_or_else(||"A previous operation is still protected by its recovery helper. Restore the saved configuration before switching.".into()),deadline_ms:None,choices:record.previous_choices.clone(),mode:record.previous_mode,hardware_verified:false});
    }
    Ok(
        serde_json::json!({"mapping":state.mapping,"active":state.active,"mode":state.mode,
        "operation":operation,"has_entry":entry(node)?.is_some(),
        "recovery":record.as_ref().and_then(|r|r.outcome.as_ref()),"imported":if state.mapping.is_none(){migration(node)}else{None},
        "baseline_required":baseline_required(node,&state),"group":group(node)?,"cooldown_until":*lock(&node.inner.physical.cooldown)}),
    )
}
pub(crate) fn resume_recovery(node: &Node) {
    if !node.native_services_enabled() {
        return;
    }
    match recovery::read(&recovery::path(node.data_dir())) {
        Ok(Some(record)) if record.outcome.is_some() => {
            let n = node.clone();
            node.inner.rt.spawn_blocking(move || {
                if let Err(error) = execute(&n, Request::Recovered { operation: record.id }) {
                    tracing::error!(%error,"completed physical recovery could not reconcile routing");
                }
            });
        }
        Ok(Some(_)) => {
            if let Err(error) = recovery::spawn(&recovery::path(node.data_dir())) {
                tracing::error!(%error,"physical display recovery could not resume");
            }
        }
        Err(error) => tracing::error!(%error,"physical display recovery is unreadable"),
        _ => {}
    }
}

fn baseline_required(node: &Node, state: &Store) -> bool {
    let Some(mapping) = &state.mapping else { return false };
    if state.active.is_empty() {
        return true;
    }
    let expected = mapping.rectangles(&state.active).ok();
    node.link_config()
        .ok()
        .is_none_or(|c| expected.as_ref() != Some(&c.applied().layout.settings.physical_screens))
}
pub fn execute(node: &Node, request: Request) -> Result<serde_json::Value, String> {
    if matches!(request, Request::Confirmation) {
        let record = recovery::read(&recovery::path(node.data_dir()))?.or(lock(&node.inner.physical.local).clone());
        let prompt = record
            .filter(|r| group(node).ok().as_deref() == Some(r.group.as_str()))
            .filter(|r| {
                r.confirmation.as_ref().is_none_or(|c| c.phase != "pending") || r.owner_boot == Some(node.boot().nonce)
            })
            .and_then(|r| r.confirmation)
            .filter(|c| {
                c.deadline_ms > recovery::now_ms()
                    && (if c.phase == "pending" {
                        c.bounds.is_some()
                    } else {
                        c.previous_bounds.is_some() || c.phase == "kept"
                    })
            });
        return serde_json::to_value(prompt).map_err(|e| e.to_string());
    }
    if let Request::Identify { device, display, label } = &request {
        return transport::request(
            node,
            *device,
            PeerRequest::Identify {
                display: display.clone(),
                label: label.clone(),
            },
        );
    }
    if let Request::Inspect { device } = &request {
        return transport::request(node, *device, PeerRequest::Inspect);
    }
    if let Request::Read { device, display } = &request {
        return transport::request(
            node,
            *device,
            PeerRequest::Read {
                display: display.clone(),
            },
        );
    }
    if let Request::RecoverLocal = &request {
        let _gate = node
            .inner
            .physical
            .local_gate
            .try_lock()
            .map_err(|_| "A display command is still finishing")?;
        let _file_guard = recovery::guard(node.data_dir())?;
        let record = entry(node)?.ok_or("There is no saved configuration to restore")?;
        restore_record(node, &record)?;
        set_route(node, record.route.clone())?;
        let mut current = store(node)?;
        current.active.clear();
        current.mode = None;
        persist(node, current)?;
        phase(
            node,
            "reverted",
            "This computer's original display configuration was restored",
            None,
        );
        return status(node);
    }
    if let Request::Recovered { operation } = &request {
        let _gate = node
            .inner
            .physical
            .local_gate
            .try_lock()
            .map_err(|_| "The display owner is busy")?;
        let _file_guard = recovery::guard(node.data_dir())?;
        let mut record = recovery::read(&recovery::path(node.data_dir()))?.ok_or("Recovery record not found")?;
        if record.id != *operation || record.outcome.is_none() {
            return Err("Recovery has not completed".into());
        }
        if record.group != group(node)? {
            return Err("Recovery belongs to a different paired group".into());
        }
        set_route(node, record.route.clone())?;
        if let Some(prompt) = record.confirmation.as_mut() {
            prompt.phase = if record.outcome.as_ref().is_some_and(Result::is_ok) {
                "reverted"
            } else {
                "error"
            }
            .into();
            prompt.deadline_ms = recovery::now_ms() + 30_000;
        }
        recovery::write(&recovery::path(node.data_dir()), &record)?;
        show_prompt(node, &record);
        *lock(&node.inner.physical.local) = Some(record.clone());
        let mut current = store(node)?;
        current.active = record.previous_choices.clone();
        current.mode = record.previous_mode;
        persist(node, current)?;
        if let Some((_, tx)) = lock(&node.inner.physical.decision).as_ref() {
            let _ = tx.send(false);
        }
        if lock(&node.inner.physical.operation).is_none() {
            *lock(&node.inner.physical.operation) = Some(Operation {
                id: record.id.clone(),
                phase: "restoring".into(),
                detail: String::new(),
                deadline_ms: None,
                choices: record.previous_choices.clone(),
                mode: record.previous_mode,
                hardware_verified: false,
            });
        }
        phase(
            node,
            if record.outcome == Some(Ok(())) {
                "reverted"
            } else {
                "needs_attention"
            },
            "The independent recovery helper handled an expired operation",
            None,
        );
        return status(node);
    }
    let owner = coordinator(node, &request)?;
    if owner.0 != node.id() {
        let result = transport::request(
            node,
            owner,
            PeerRequest::Client {
                request: Box::new(request.clone()),
            },
        );
        if matches!(request, Request::Status) && result.is_err() {
            let mut local = status(node)?;
            local["coordinator_error"] = serde_json::Value::String(result.err().unwrap());
            return Ok(local);
        }
        return result;
    }
    match request {
        Request::Status => status(node),
        Request::RecoverCalibration { operation } => {
            if busy(node) || entry(node)?.is_some() {
                return Err("Finish the active operation or restore its full session first".into());
            }
            let _gate = node
                .inner
                .physical
                .local_gate
                .try_lock()
                .map_err(|_| "The display owner is busy")?;
            let _file_guard = recovery::guard(node.data_dir())?;
            let mut record =
                recovery::read(&recovery::path(node.data_dir()))?.ok_or("No calibration recovery is available")?;
            if record.id != operation
                || record.group != group(node)?
                || record.desktop.is_some()
                || record.inputs.len() != 1
            {
                return Err("This recovery does not belong to the pending calibration".into());
            }
            let outcome = restore_record(node, &record);
            record.outcome = Some(outcome.clone());
            recovery::write(&recovery::path(node.data_dir()), &record)?;
            match outcome {
                Ok(()) => phase(
                    node,
                    "reverted",
                    "The input saved before calibration was restored",
                    None,
                ),
                Err(error) => phase(node, "needs_attention", error, None),
            }
            status(node)
        }
        Request::Save {
            expected_revision,
            mut mapping,
        } => {
            let _gate = node
                .inner
                .physical
                .local_gate
                .try_lock()
                .map_err(|_| "Another operation is in progress")?;
            if busy(node) || !store(node)?.active.is_empty() || entry(node)?.is_some() {
                return Err("Restore the original configuration before changing bindings".into());
            }
            if store(node)?.mapping.as_ref().map_or(0, |m| m.revision) != expected_revision {
                return Err("Setup changed; refresh before saving".into());
            }
            mapping.group = group(node)?;
            mapping.coordinator = node.id().into();
            mapping.revision = expected_revision
                .checked_add(1)
                .ok_or("Configuration revision exhausted")?;
            mapping.validate()?;
            let peers = participants(node, &mapping)?;
            let previous = store(node)?.mapping;
            let saved = (|| {
                for peer in peers.iter().filter(|p| p.0 != node.id()) {
                    transport::request(
                        node,
                        *peer,
                        PeerRequest::Install {
                            expected_revision,
                            mapping: mapping.clone(),
                        },
                    )?;
                }
                persist(
                    node,
                    Store {
                        mapping: Some(mapping.clone()),
                        ..Default::default()
                    },
                )
            })();
            if let Err(error) = saved {
                let mut errors = Vec::new();
                // Include an unacknowledged recipient: it may have saved before
                // its reply was lost. Revision checks cannot undo a later edit.
                for peer in peers.iter().filter(|p| p.0 != node.id()) {
                    if let Err(e) = transport::request(
                        node,
                        *peer,
                        PeerRequest::UndoInstall {
                            revision: mapping.revision,
                            previous: previous.clone(),
                        },
                    ) {
                        errors.push(e);
                    }
                }
                return Err(if errors.is_empty() {
                    error
                } else {
                    format!("{error}. Setup synchronization needs attention: {}", errors.join("; "))
                });
            }
            status(node)
        }
        Request::Calibrate { panel, target } => {
            if recovery::now_ms() < *lock(&node.inner.physical.cooldown) {
                return Err("Let the displays settle before starting another calibration".into());
            }
            if !panel.endpoints.iter().any(|e| e.screen.device == target) {
                return Err("The selected computer is not connected to this monitor".into());
            }
            start(node, vec![], None, move |n, operation| {
                calibrate(n, operation, panel, target)
            })?;
            status(node)
        }
        Request::Apply {
            expected_revision,
            choices,
            mode,
            baseline_confirmed,
        } => {
            let mapping = require_mapping(node)?;
            if recovery::read(&recovery::path(node.data_dir()))?.is_some_and(|r| r.outcome.is_none()) {
                return Err("Finish or restore the previous display operation first".into());
            }
            if recovery::read(&recovery::path(node.data_dir()))?
                .is_some_and(|r| r.outcome.as_ref().is_some_and(Result::is_err))
            {
                return Err("Recover the previous display configuration before switching again".into());
            }
            let overview = crate::api::AgentApi::overview(node).map_err(|e| e.message().to_string())?;
            if overview.focus_lock.is_some() {
                return Err("Unlock pointer focus before switching physical monitors".into());
            }
            let config = node.link_config().map_err(|e| e.message().to_string())?;
            let applied = config.applied();
            if applied.paused
                || choices
                    .iter()
                    .any(|s| applied.owners.get(&s.device.0).is_some_and(|o| !o.screen_shared))
            {
                return Err("Enable screen and input sharing on the selected computers first".into());
            }
            if mapping.revision != expected_revision {
                return Err("Setup changed; refresh before switching".into());
            }
            if lock(&node.inner.physical.operation)
                .as_ref()
                .is_some_and(|o| o.phase == "needs_attention")
            {
                return Err("Recover the previous operation before switching again".into());
            }
            mapping.rectangles(&choices)?;
            if mapping.panels.iter().any(|p| !p.calibrated) {
                return Err("Confirm every monitor's calibration first".into());
            }
            if baseline_required(node, &store(node)?)
                && mapping.panels.iter().any(|p| !p.readback)
                && !baseline_confirmed
            {
                return Err("Confirm the current picture before entering physical mode".into());
            }
            if !baseline_required(node, &store(node)?)
                && store(node)?.active == choices
                && store(node)?.mode == Some(mode)
            {
                return status(node);
            }
            if recovery::now_ms() < *lock(&node.inner.physical.cooldown) {
                return Err("Let the displays settle before switching again".into());
            }
            participants(node, &mapping)?;
            start(node, choices.clone(), Some(mode), move |n, operation| {
                apply(n, operation, mapping, choices, mode)
            })?;
            status(node)
        }
        Request::Keep { ref operation } | Request::Revert { ref operation } => {
            let keep = matches!(request, Request::Keep { .. });
            let op = lock(&node.inner.physical.operation)
                .clone()
                .ok_or("There is no operation to confirm")?;
            if op.id != *operation
                || !(op.phase == "awaiting_confirmation" || !keep && op.phase == "calibrating")
                || recovery::now_ms() >= op.deadline_ms.unwrap_or(0)
            {
                return Err("This confirmation has expired".into());
            }
            let decision = lock(&node.inner.physical.decision);
            let (id, tx) = decision.as_ref().ok_or("This operation already ended")?;
            if id != operation {
                return Err("This confirmation belongs to an older operation".into());
            }
            tx.send(keep).map_err(|_| "The operation already ended")?;
            status(node)
        }
        Request::Restore => {
            let mapping = require_mapping(node)?;
            start(node, vec![], None, move |n, _operation| restore_session(n, &mapping))?;
            status(node)
        }
        Request::Confirmation
        | Request::Identify { .. }
        | Request::Inspect { .. }
        | Request::Read { .. }
        | Request::RecoverLocal
        | Request::Recovered { .. } => {
            unreachable!()
        }
    }
}
fn start(
    node: &Node,
    choices: Vec<ScreenKey>,
    mode: Option<Mode>,
    work: impl FnOnce(&Node, String) -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    let _gate = node
        .inner
        .physical
        .local_gate
        .try_lock()
        .map_err(|_| "Another operation is in progress")?;
    let mut current = lock(&node.inner.physical.operation);
    if current.as_ref().is_some_and(Operation::busy) {
        return Err("Finish or recover the previous operation first".into());
    }
    let operation = id();
    *current = Some(Operation {
        id: operation.clone(),
        phase: "checking".into(),
        detail: String::new(),
        deadline_ms: None,
        choices,
        mode,
        hardware_verified: false,
    });
    let n = node.clone();
    node.inner.rt.spawn_blocking(move || {
        if let Err(error) = work(&n, operation)
            && lock(&n.inner.physical.operation)
                .as_ref()
                .is_none_or(|o| o.phase != "needs_attention")
        {
            phase(&n, "failed", error, None);
        }
        *lock(&n.inner.physical.cooldown) = recovery::now_ms() + COOLDOWN_MS;
    });
    Ok(())
}
fn authorize(node: &Node, peer: DeviceKey) -> Result<(), String> {
    if peer.0 == node.id() {
        return Ok(());
    }
    if require_mapping(node)?.coordinator != peer {
        return Err("Only this setup's coordinator can change displays".into());
    }
    Ok(())
}
pub(super) fn receive(node: &Node, peer: DeviceKey, request: PeerRequest) -> Result<serde_json::Value, String> {
    match request {
        PeerRequest::Client { request } => {
            if matches!(*request, Request::RecoverLocal | Request::Recovered { .. }) {
                return Err("This operation must be requested by the local user".into());
            }
            if let Some(mapping) = store(node)?.mapping
                && (mapping.coordinator.0 != node.id() || !mapping.participants().contains(&peer))
            {
                return Err("The sender is not a participant of this setup".into());
            }
            execute(node, *request)
        }
        PeerRequest::Identify { display, label } => {
            if label.len() > 128 {
                return Err("Monitor label is too long".into());
            }
            let displays = node
                .inner
                .display_backend
                .as_ref()
                .ok_or("Display backend unavailable")?
                .displays()
                .map_err(|e| e.to_string())?;
            if !displays.iter().any(|d| d.key == display && !d.is_virtual) {
                return Err("The selected display is not connected".into());
            }
            crate::display_overlay::identify_physical(node, display, label);
            Ok(serde_json::json!({"identified":true}))
        }
        PeerRequest::Inspect => {
            let displays = if node.native_services_enabled() {
                desktop::displays()?
            } else {
                node.inner
                    .display_backend
                    .as_ref()
                    .ok_or("Display backend unavailable")?
                    .displays()
                    .map_err(|e| e.to_string())?
            };
            Ok(
                serde_json::json!({"device":DeviceKey(node.id()),"displays":displays,"desktop_supported":node.native_services_enabled()&&desktop::available(),"ddc_supported":node.native_services_enabled()&&cfg!(any(target_os="macos",windows)),"desktop_verified":false}),
            )
        }
        PeerRequest::Read { display } => {
            let displays = node
                .inner
                .display_backend
                .as_ref()
                .ok_or("Display backend unavailable")?
                .displays()
                .map_err(|e| e.to_string())?;
            if !displays.iter().any(|d| d.key == display && !d.is_virtual) {
                return Err("Choose an exact connected physical display".into());
            }
            Ok(serde_json::json!({"input":hardware(node).read(&display)?}))
        }
        PeerRequest::Install {
            expected_revision,
            mapping,
        } => {
            mapping.validate()?;
            if mapping.group != group(node)?
                || mapping.coordinator != peer
                || !mapping.participants().contains(&node.id().into())
            {
                return Err("Invalid setup authority or group".into());
            }
            let _gate = node
                .inner
                .physical
                .local_gate
                .try_lock()
                .map_err(|_| "The display owner is busy")?;
            let _file_guard = recovery::guard(node.data_dir())?;
            let current = store(node)?;
            if current.mapping.as_ref() == Some(&mapping) {
                return Ok(serde_json::json!({"saved":true}));
            }
            if current.mapping.as_ref().is_some_and(|m| m.coordinator != peer) {
                return Err("This computer already belongs to a different physical setup coordinator".into());
            }
            if busy(node)
                || !current.active.is_empty()
                || entry(node)?.is_some()
                || current.mapping.as_ref().map_or(0, |m| m.revision) > expected_revision
            {
                return Err("This computer's setup changed; refresh before saving".into());
            }
            persist(
                node,
                Store {
                    mapping: Some(mapping),
                    ..Default::default()
                },
            )?;
            Ok(serde_json::json!({"saved":true}))
        }
        PeerRequest::UndoInstall { revision, previous } => {
            let _gate = node
                .inner
                .physical
                .local_gate
                .try_lock()
                .map_err(|_| "The setup owner is busy")?;
            let current = store(node)?;
            if current.mapping.is_none() {
                return Ok(serde_json::json!({"restored":true}));
            }
            authorize(node, peer)?;
            if current.mapping.as_ref().is_some_and(|m| m.revision == revision)
                && current.active.is_empty()
                && entry(node)?.is_none()
            {
                if let Some(mapping) = &previous {
                    mapping.validate()?;
                    if mapping.coordinator != peer || mapping.group != group(node)? {
                        return Err("Invalid prior setup authority".into());
                    }
                }
                persist(
                    node,
                    Store {
                        mapping: previous,
                        ..Default::default()
                    },
                )?;
            }
            Ok(serde_json::json!({"restored":true}))
        }
        PeerRequest::Publish {
            revision,
            choices,
            mode,
        } => {
            authorize(node, peer)?;
            let mut current = store(node)?;
            let mapping = current.mapping.as_ref().ok_or("Setup unavailable")?;
            if mapping.revision != revision {
                return Err("Setup revision changed during the switch".into());
            }
            if !choices.is_empty() {
                mapping.rectangles(&choices)?;
            }
            current.mode = if choices.is_empty() { None } else { Some(mode) };
            current.active = choices;
            persist(node, current)?;
            Ok(serde_json::json!({"saved":true}))
        }
        PeerRequest::Prepare {
            operation,
            inputs,
            mode,
            plan,
        } => {
            authorize(node, peer)?;
            if operation.len() != 32 || !operation.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("Invalid operation identity".into());
            }
            if peer.0 != node.id() && !inputs.is_empty() {
                return Err("Remote owners cannot replace this computer's input recovery commands".into());
            }
            let _gate = node
                .inner
                .physical
                .local_gate
                .try_lock()
                .map_err(|_| "The display owner is busy")?;
            let _file_guard = recovery::guard(node.data_dir())?;
            if let Some(existing) = recovery::read(&recovery::path(node.data_dir()))? {
                if existing.outcome.is_none() {
                    if existing.id == operation {
                        return Ok(serde_json::json!({"prepared":true}));
                    }
                    return Err("A prior physical display operation still needs recovery".into());
                }
                if existing.outcome.as_ref().is_some_and(|r| r.is_err()) {
                    return Err("Recover the previous display operation first".into());
                }
            }
            let old_entry = entry(node)?;
            let native = if mode == Mode::Complete || old_entry.as_ref().is_some_and(|r| r.desktop.is_some()) {
                Some(hardware(node).capture()?)
            } else {
                None
            };
            if mode == Mode::Complete {
                let base = old_entry
                    .as_ref()
                    .and_then(|r| r.desktop.as_ref())
                    .or(native.as_ref())
                    .ok_or("Desktop adapter unavailable")?;
                let retained = plan.retained(&base.displays.iter().map(|d| d.key.clone()).collect::<Vec<_>>())?;
                if !base.displays.iter().any(|d| !d.is_virtual && retained.contains(&d.key)) {
                    return Err("Retain one physical display or use compatible mode".into());
                }
                hardware(node).preflight(base, &plan)?;
            }
            let record = Record {
                id: operation,
                deadline_ms: recovery::now_ms() + GUARD_MS,
                desktop: native,
                inputs,
                route: node
                    .link_config()
                    .map_err(|e| e.message().to_string())?
                    .applied()
                    .layout
                    .settings
                    .physical_screens
                    .clone(),
                outcome: None,
                ready: false,
                helper_pid: None,
                owner_boot: Some(node.boot().nonce),
                group: group(node)?,
                previous_choices: store(node)?.active,
                previous_mode: store(node)?.mode,
                visible_displays: plan.visible.clone(),
                prompt_displays: node
                    .inner
                    .display_backend
                    .as_ref()
                    .ok_or("Display backend unavailable")?
                    .displays()
                    .map_err(|e| e.to_string())?,
                confirmation: None,
            };
            if old_entry.is_none() {
                recovery::write(&root(node).join("entry-recovery.json"), &record)?;
            } else if old_entry.as_ref().is_some_and(|r| r.desktop.is_none()) && record.desktop.is_some() {
                let mut old = old_entry.clone().unwrap();
                old.desktop = record.desktop.clone();
                recovery::write(&root(node).join("entry-recovery.json"), &old)?;
            }
            recovery::write(&recovery::path(node.data_dir()), &record)?;
            *lock(&node.inner.physical.local) = Some(record);
            if node.native_services_enabled()
                && let Err(error) = recovery::spawn(&recovery::path(node.data_dir()))
            {
                let _ = std::fs::remove_file(recovery::path(node.data_dir()));
                if old_entry.is_none() {
                    let _ = std::fs::remove_file(root(node).join("entry-recovery.json"));
                }
                return Err(error);
            }
            Ok(serde_json::json!({"prepared":true}))
        }
        PeerRequest::Connect { operation } => {
            authorize(node, peer)?;
            let _gate = node
                .inner
                .physical
                .local_gate
                .try_lock()
                .map_err(|_| "The display owner is busy")?;
            let _file_guard = recovery::guard(node.data_dir())?;
            same_operation(node, &operation)?;
            if let Some(snapshot) = entry(node)?.and_then(|r| r.desktop) {
                hardware(node).restore(&snapshot)?;
            }
            Ok(serde_json::json!({"connected":true}))
        }
        PeerRequest::Desktop { operation, plan } => {
            authorize(node, peer)?;
            let _gate = node
                .inner
                .physical
                .local_gate
                .try_lock()
                .map_err(|_| "The display owner is busy")?;
            let _file_guard = recovery::guard(node.data_dir())?;
            same_operation(node, &operation)?;
            let mapping = require_mapping(node)?;
            let managed: std::collections::BTreeSet<_> = mapping
                .panels
                .iter()
                .flat_map(|p| p.endpoints.iter())
                .filter(|e| e.screen.device.0 == node.id())
                .map(|e| e.screen.display.clone())
                .collect();
            if plan.managed.iter().cloned().collect::<std::collections::BTreeSet<_>>() != managed {
                return Err("A physical layout cannot change unmanaged displays".into());
            }
            let snapshot = entry(node)?
                .and_then(|r| r.desktop)
                .ok_or("Native desktop snapshot unavailable")?;
            hardware(node).apply(&snapshot, &plan)?;
            Ok(serde_json::json!({"applied":true}))
        }
        PeerRequest::Await { operation, deadline_ms } => {
            authorize(node, peer)?;
            let _gate = node
                .inner
                .physical
                .local_gate
                .try_lock()
                .map_err(|_| "The display owner is busy")?;
            let _file_guard = recovery::guard(node.data_dir())?;
            let mut record = same_operation(node, &operation)?;
            if deadline_ms <= recovery::now_ms() || deadline_ms > recovery::now_ms() + CONFIRM_MS + 1000 {
                return Err("Invalid confirmation deadline".into());
            }
            record.deadline_ms = deadline_ms;
            record.confirmation = Some(make_confirmation(node, &record, "pending", deadline_ms)?);
            recovery::write(&recovery::path(node.data_dir()), &record)?;
            show_prompt(node, &record);
            *lock(&node.inner.physical.local) = Some(record);
            Ok(serde_json::json!({"armed":true}))
        }
        PeerRequest::Finish { operation } => {
            authorize(node, peer)?;
            let _gate = node
                .inner
                .physical
                .local_gate
                .try_lock()
                .map_err(|_| "The display owner is busy")?;
            let _file_guard = recovery::guard(node.data_dir())?;
            let mut record = same_operation(node, &operation)?;
            if let Some(prompt) = record.confirmation.as_mut() {
                prompt.phase = "kept".into();
                prompt.deadline_ms = recovery::now_ms() + 30_000;
            }
            std::fs::remove_file(recovery::path(node.data_dir())).map_err(|e| e.to_string())?;
            *lock(&node.inner.physical.local) = Some(record);
            Ok(serde_json::json!({"confirmed":true}))
        }
        PeerRequest::Rollback { operation } => {
            authorize(node, peer)?;
            let _gate = node
                .inner
                .physical
                .local_gate
                .try_lock()
                .map_err(|_| "The display owner is busy")?;
            let _file_guard = recovery::guard(node.data_dir())?;
            let mut record = recovery::read(&recovery::path(node.data_dir()))?
                .or(lock(&node.inner.physical.local).clone())
                .ok_or("Recovery snapshot unavailable")?;
            if record.id != operation {
                return Err("Recovery request belongs to an older operation".into());
            }
            let outcome = if record.outcome == Some(Ok(())) {
                Ok(())
            } else {
                restore_record(node, &record)
            };
            record.outcome = Some(outcome.clone());
            if let Some(prompt) = record.confirmation.as_mut() {
                prompt.phase = if outcome.is_ok() { "reverted" } else { "error" }.into();
                prompt.deadline_ms = recovery::now_ms() + 30_000;
            }
            recovery::write(&recovery::path(node.data_dir()), &record)?;
            show_prompt(node, &record);
            *lock(&node.inner.physical.local) = Some(record);
            outcome?;
            Ok(serde_json::json!({"restored":true}))
        }
        PeerRequest::RestoreEntry => {
            authorize(node, peer)?;
            let _gate = node
                .inner
                .physical
                .local_gate
                .try_lock()
                .map_err(|_| "The display owner is busy")?;
            let _file_guard = recovery::guard(node.data_dir())?;
            let mut restored_prompt = None;
            if let Some(mut record) = entry(node)? {
                restore_record(node, &record)?;
                record.confirmation = Some(make_confirmation(
                    node,
                    &record,
                    "reverted",
                    recovery::now_ms() + 30_000,
                )?);
                record.outcome = Some(Ok(()));
                restored_prompt = Some(record);
                std::fs::remove_file(root(node).join("entry-recovery.json")).map_err(|e| e.to_string())?;
            }
            if recovery::path(node.data_dir()).exists() {
                std::fs::remove_file(recovery::path(node.data_dir())).map_err(|e| e.to_string())?;
            }
            let mut current = store(node)?;
            current.active.clear();
            current.mode = None;
            persist(node, current)?;
            if let Some(record) = restored_prompt {
                *lock(&node.inner.physical.local) = Some(record.clone());
                show_prompt(node, &record);
            }
            Ok(serde_json::json!({"restored":true}))
        }
    }
}

fn make_confirmation(node: &Node, record: &Record, phase: &str, deadline_ms: u64) -> Result<Confirmation, String> {
    let mapping = require_mapping(node)?;
    let previous = if record.previous_choices.is_empty() {
        mapping.baseline_choices()
    } else {
        record.previous_choices.clone()
    };
    let previous_bounds = previous.iter().filter(|s| s.device.0 == node.id()).find_map(|s| {
        record
            .prompt_displays
            .iter()
            .find(|d| d.key == s.display)
            .map(|d| d.bounds)
    });
    let bounds = if phase == "pending" && !record.visible_displays.is_empty() {
        let displays = node
            .inner
            .display_backend
            .as_ref()
            .ok_or("Display backend unavailable")?
            .displays()
            .map_err(|e| e.to_string())?;
        Some(
            record
                .visible_displays
                .iter()
                .find_map(|key| displays.iter().find(|d| &d.key == key).map(|d| d.bounds))
                .ok_or("No assigned display is available for layout confirmation")?,
        )
    } else {
        None
    };
    Ok(Confirmation {
        operation: record.id.clone(),
        phase: phase.into(),
        deadline_ms,
        bounds,
        previous_bounds,
        macos: cfg!(target_os = "macos"),
    })
}
fn show_prompt(node: &Node, record: &Record) {
    if node.native_services_enabled()
        && record.confirmation.as_ref().is_some_and(|c| {
            if c.phase == "pending" {
                c.bounds.is_some()
            } else {
                c.phase != "kept" && c.previous_bounds.is_some()
            }
        })
    {
        node.emit(crate::api::ApiEvent::ShowWindow);
    }
}
fn set_route(node: &Node, route: Vec<oink_display::layout::PhysicalScreen>) -> Result<(), String> {
    let started = std::time::Instant::now();
    loop {
        match node.commit_manual_layout(|layout| {
            layout.settings.physical_screens = route.clone();
            Ok(())
        }) {
            Ok(()) => return Ok(()),
            // This refusal occurs before the layout owner writes anything.
            // Native operations are never replayed; only this idempotent routing
            // assignment waits for a short, already-running catalog transaction.
            Err(error)
                if error.message() == "Layout operation in progress" && started.elapsed() < Duration::from_secs(2) =>
            {
                std::thread::sleep(Duration::from_millis(20))
            }
            Err(error) => return Err(error.message().to_string()),
        }
    }
}
fn previous_inputs(node: &Node, mapping: &Mapping) -> Result<Vec<recovery::Input>, String> {
    let current = store(node)?;
    let active = if !baseline_required(node, &current) && current.active.len() == mapping.panels.len() {
        current.active
    } else {
        mapping.baseline_choices()
    };
    mapping
        .panels
        .iter()
        .zip(active)
        .map(|(panel, key)| {
            let controller = panel
                .endpoints
                .iter()
                .find(|e| e.screen.device == mapping.coordinator)
                .ok_or("Coordinator connection unavailable")?;
            let previous = panel
                .endpoints
                .iter()
                .find(|e| e.screen == key)
                .ok_or("Current picture needs confirmation")?;
            Ok(recovery::Input {
                display: controller.screen.display.clone(),
                value: if panel.readback {
                    hardware(node).read(&controller.screen.display)?
                } else {
                    previous.input
                },
                readback: panel.readback,
            })
        })
        .collect()
}

fn apply(node: &Node, operation: String, mapping: Mapping, choices: Vec<ScreenKey>, mode: Mode) -> Result<(), String> {
    let peers = participants(node, &mapping)?;
    let before = store(node)?;
    let route = node
        .link_config()
        .map_err(|e| e.message().to_string())?
        .applied()
        .layout
        .settings
        .physical_screens
        .clone();
    let inputs = previous_inputs(node, &mapping)?;
    let mut prepared = Vec::new();
    let result: Result<(), String> = (|| {
        phase(
            node,
            "preparing",
            "Saving recovery points on the participating computers",
            None,
        );
        for peer in &peers {
            transport::request(
                node,
                *peer,
                PeerRequest::Prepare {
                    operation: operation.clone(),
                    inputs: if peer.0 == node.id() { inputs.clone() } else { vec![] },
                    mode,
                    plan: DesktopPlan::for_device(&mapping, &choices, *peer),
                },
            )?;
            prepared.push(*peer);
        }
        phase(node, "connecting", "Reconnecting the target displays", None);
        for peer in &peers {
            transport::request(
                node,
                *peer,
                PeerRequest::Connect {
                    operation: operation.clone(),
                },
            )?;
        }
        phase(
            node,
            "switching",
            "Switching monitor inputs and allowing links to settle",
            None,
        );
        for ((panel, key), old) in mapping.panels.iter().zip(&choices).zip(&inputs) {
            let selected = panel
                .endpoints
                .iter()
                .find(|e| &e.screen == key)
                .ok_or("Selected connection unavailable")?;
            let current = if panel.readback {
                hardware(node).read(&old.display)?
            } else {
                old.value
            };
            if selected.input != current
                || !panel.readback && (mode == Mode::Complete || entry(node)?.is_some_and(|r| r.desktop.is_some()))
            {
                let _guard = recovery::guard(node.data_dir())?;
                same_operation(node, &operation)?;
                hardware(node).write(&old.display, selected.input, panel.readback)?;
            }
        }
        if mode == Mode::Complete {
            phase(node, "desktop", "Updating visible desktops and main displays", None);
            for peer in &peers {
                transport::request(
                    node,
                    *peer,
                    PeerRequest::Desktop {
                        operation: operation.clone(),
                        plan: DesktopPlan::for_device(&mapping, &choices, *peer),
                    },
                )?;
            }
        }
        let rectangles = mapping.rectangles(&choices)?;
        set_route(node, rectangles)?;
        #[cfg(test)]
        let confirmation = match node
            .inner
            .physical
            .confirm_ms
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            0 => CONFIRM_MS,
            n => n,
        };
        #[cfg(not(test))]
        let confirmation = CONFIRM_MS;
        let deadline = recovery::now_ms() + confirmation;
        let (tx, rx) = mpsc::channel();
        *lock(&node.inner.physical.decision) = Some((operation.clone(), tx));
        phase(
            node,
            "awaiting_confirmation",
            "Keep this layout or it will automatically return",
            Some(deadline),
        );
        for peer in &peers {
            transport::request(
                node,
                *peer,
                PeerRequest::Await {
                    operation: operation.clone(),
                    deadline_ms: deadline,
                },
            )?;
        }
        if !rx
            .recv_timeout(Duration::from_millis(deadline.saturating_sub(recovery::now_ms())))
            .unwrap_or(false)
        {
            return Err("The layout was cancelled or not confirmed in time".into());
        }
        participants(node, &mapping)?;
        // Publish only after all desktop owners applied and a visible client kept it.
        for peer in &peers {
            transport::request(
                node,
                *peer,
                PeerRequest::Publish {
                    revision: mapping.revision,
                    choices: choices.clone(),
                    mode,
                },
            )?;
        }
        for peer in &peers {
            transport::request(
                node,
                *peer,
                PeerRequest::Finish {
                    operation: operation.clone(),
                },
            )?;
        }
        if let Some(op) = lock(&node.inner.physical.operation).as_mut() {
            op.hardware_verified = mapping.panels.iter().all(|p| p.readback);
        }
        phase(
            node,
            "completed",
            if mode == Mode::Compatible {
                "Layout applied in compatible mode; operating systems retain their display spaces"
            } else {
                "Monitor inputs, visible desktops and main displays applied"
            },
            None,
        );
        if let Some(first) = choices.first() {
            let _ = node
                .input_thread()
                .map(|i| i.send(crate::threads::input::InputCommand::MovePointerTo(first.device.0)));
        }
        Ok(())
    })();
    *lock(&node.inner.physical.decision) = None;
    if let Err(error) = result {
        phase(
            node,
            "restoring",
            "Restoring the previous monitor and desktop configuration",
            None,
        );
        let mut failures = Vec::new();
        for peer in prepared.iter().rev() {
            if let Err(e) = transport::request(
                node,
                *peer,
                PeerRequest::Rollback {
                    operation: operation.clone(),
                },
            ) {
                failures.push(e);
            }
        }
        if let Err(e) = set_route(node, route) {
            failures.push(e);
        }
        for peer in &prepared {
            if let Err(e) = transport::request(
                node,
                *peer,
                PeerRequest::Publish {
                    revision: mapping.revision,
                    choices: before.active.clone(),
                    mode: before.mode.unwrap_or(Mode::Compatible),
                },
            ) {
                failures.push(e);
            }
        }
        if let Err(e) = persist(node, before) {
            failures.push(e);
        }
        if failures.is_empty() {
            phase(
                node,
                "reverted",
                format!(
                    "{error}. The previous configuration was restored; monitors without readback use the calibrated restoration input"
                ),
                None,
            );
        } else {
            phase(
                node,
                "needs_attention",
                format!("{error}. Recovery needs attention: {}", failures.join("; ")),
                None,
            );
        }
    }
    Ok(())
}
fn calibrate(node: &Node, operation: String, panel: Panel, target: DeviceKey) -> Result<(), String> {
    Mapping {
        revision: 0,
        group: group(node)?,
        coordinator: node.id().into(),
        panels: vec![panel.clone()],
    }
    .validate()?;
    let controller = panel
        .endpoints
        .iter()
        .find(|e| e.screen.device.0 == node.id())
        .ok_or("Run calibration from a computer connected to this monitor")?;
    let selected = panel
        .endpoints
        .iter()
        .find(|e| e.screen.device == target)
        .ok_or("Target input unavailable")?;
    let baseline = panel
        .endpoints
        .iter()
        .find(|e| e.screen.device == panel.baseline)
        .ok_or("Confirm the current picture before testing")?;
    if [baseline.input, selected.input].contains(&0) {
        return Err("Choose nonzero input values before testing".into());
    }
    let displays = node
        .inner
        .display_backend
        .as_ref()
        .ok_or("Display backend unavailable")?
        .displays()
        .map_err(|e| e.to_string())?;
    if !displays
        .iter()
        .any(|d| d.key == controller.screen.display && !d.is_virtual && !d.is_builtin)
    {
        return Err("Choose a connected external physical display".into());
    }
    let target_state = transport::request(node, target, PeerRequest::Inspect)?;
    if !target_state["displays"].as_array().is_some_and(|displays| {
        displays
            .iter()
            .any(|d| d["key"].as_str() == Some(selected.screen.display.as_str()))
    }) {
        return Err("The target computer does not currently detect this display".into());
    }
    if busy_entry(node)? {
        return Err("Restore the active physical layout before calibrating".into());
    }
    let previous = if panel.readback {
        hardware(node).read(&controller.screen.display)?
    } else {
        baseline.input
    };
    let record = Record {
        id: operation,
        deadline_ms: recovery::now_ms() + 30_000,
        desktop: None,
        inputs: vec![recovery::Input {
            display: controller.screen.display.clone(),
            value: previous,
            readback: panel.readback,
        }],
        route: vec![],
        outcome: None,
        ready: false,
        helper_pid: None,
        owner_boot: Some(node.boot().nonce),
        group: group(node)?,
        previous_choices: store(node)?.active,
        previous_mode: store(node)?.mode,
        visible_displays: vec![],
        prompt_displays: vec![],
        confirmation: None,
    };
    recovery::write(&recovery::path(node.data_dir()), &record)?;
    if node.native_services_enabled() {
        recovery::spawn(&recovery::path(node.data_dir()))?;
    }
    let result: Result<(), String> = (|| {
        {
            let _guard = recovery::guard(node.data_dir())?;
            hardware(node).write(&controller.screen.display, selected.input, panel.readback)?;
        }
        transport::request(
            node,
            target,
            PeerRequest::Identify {
                display: selected.screen.display.clone(),
                label: panel.name.clone(),
            },
        )?;
        let (tx, rx) = mpsc::channel();
        *lock(&node.inner.physical.decision) = Some((record.id.clone(), tx));
        phase(
            node,
            "calibrating",
            "Showing the selected computer briefly; the original input will return automatically",
            Some(recovery::now_ms() + 10_000),
        );
        let _ = rx.recv_timeout(hardware(node).calibration_duration());
        Ok(())
    })();
    *lock(&node.inner.physical.decision) = None;
    let restored = {
        let _guard = recovery::guard(node.data_dir())?;
        restore_record(node, &record)
    };
    let mut record = record;
    record.outcome = Some(restored.clone());
    recovery::write(&recovery::path(node.data_dir()), &record)?;
    if let Err(e) = restored {
        phase(
            node,
            "needs_attention",
            format!("Calibration recovery needs attention: {e}"),
            None,
        );
        return Ok(());
    }
    result?;
    phase(
        node,
        "calibration_ready",
        "The original input was restored. Confirm whether the expected computer appeared",
        None,
    );
    Ok(())
}
fn busy_entry(node: &Node) -> Result<bool, String> {
    Ok(!store(node)?.active.is_empty() || entry(node)?.is_some())
}
fn restore_session(node: &Node, mapping: &Mapping) -> Result<(), String> {
    phase(
        node,
        "restoring",
        "Restoring the configuration saved before physical mode",
        None,
    );
    let route = entry(node)?.map(|r| r.route).unwrap_or_default();
    let mut failures = Vec::new();
    for peer in mapping.participants().iter().rev() {
        if let Err(e) = transport::request(node, *peer, PeerRequest::RestoreEntry) {
            failures.push(e);
        }
    }
    set_route(node, route)?;
    if failures.is_empty() {
        phase(
            node,
            "reverted",
            "The original monitor inputs, desktops and routing were restored",
            None,
        );
    } else {
        phase(
            node,
            "needs_attention",
            format!("Recovery needs attention: {}", failures.join("; ")),
            None,
        );
    }
    Ok(())
}
