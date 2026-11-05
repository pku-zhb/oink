//! Local physical-monitor setup and verified DDC / shared-routing transactions.
//! Controllers use BetterDisplay or native Windows DDC; every peer compiles the same visible rectangles.
use crate::{
    api::{AgentApi, ApiError, ApiResult, ScreenKey},
    node::Node,
};
use oink_display::layout::{Layout, PhysicalScreen};
use oink_foundation::Rect;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::Mutex, time::Duration};

#[cfg(windows)]
mod windows_ddc;

#[cfg(not(windows))]
use std::{
    io::Read,
    process::{Command as Process, Stdio},
    time::Instant,
};

pub const CAP: &str = "oink.physical-monitors/1";
static GATE: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub screen: ScreenKey,
    pub input: u16,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Monitor {
    pub name: String,
    /// Exact native display key: macOS UUID or Windows device-interface path.
    pub controller_display: String,
    pub bindings: Vec<Binding>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Setup {
    pub revision: u64,
    pub monitors: Vec<Monitor>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Query,
    Save {
        expected_revision: u64,
        monitors: Vec<Monitor>,
    },
    Read {
        controller_display: String,
    },
    Test {
        controller_display: String,
        input: u16,
    },
    Apply {
        expected_revision: u64,
        screens: Vec<ScreenKey>,
    },
    Clear,
}
fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::InvalidState {
        message: message.into(),
    }
}
fn uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}
fn control_identity(value: &str) -> bool {
    if cfg!(windows) {
        !value.is_empty() && value.len() <= 512
    } else {
        uuid(value)
    }
}
fn validate(monitors: &[Monitor]) -> ApiResult<()> {
    if monitors.len() > 8 {
        return Err(invalid("At most eight physical monitors"));
    }
    let mut displays = BTreeSet::new();
    let mut screens = BTreeSet::new();
    for m in monitors {
        if m.name.trim().is_empty()
            || m.name.len() > 128
            || !control_identity(&m.controller_display)
            || !displays.insert(m.controller_display.to_ascii_lowercase())
            || m.bindings.is_empty()
            || m.bindings.len() > 8
        {
            return Err(invalid(
                "Each physical monitor needs a name, a unique controller display identity and computer bindings",
            ));
        }
        let mut computers = BTreeSet::new();
        let mut inputs = BTreeSet::new();
        for b in &m.bindings {
            if b.input == 0
                || b.screen.display.is_empty()
                || b.screen.display.len() > 512
                || !computers.insert(b.screen.device)
                || !inputs.insert(b.input)
                || !screens.insert(b.screen.clone())
            {
                return Err(invalid(
                    "Bind each logical screen once, with one distinct input value per computer",
                ));
            }
        }
    }
    Ok(())
}

/// Build only the visible physical rectangles; native desktop coordinates stay intact.
fn plan(
    setup: &Setup,
    selected: &[ScreenKey],
    layout: &Layout,
    controller: crate::api::DeviceKey,
) -> ApiResult<Vec<PhysicalScreen>> {
    validate(&setup.monitors)?;
    if selected.len() != setup.monitors.len() || selected.is_empty() {
        return Err(invalid("Select one computer for every physical monitor"));
    }
    let mut x = 0.0;
    let mut visible = Vec::new();
    for (monitor, screen) in setup.monitors.iter().zip(selected) {
        if !monitor.bindings.iter().any(|b| &b.screen == screen) {
            return Err(invalid("The selected screen is not bound to this physical monitor"));
        }
        let native = layout
            .screen(&screen.into())
            .ok_or_else(|| invalid("A selected screen is no longer connected"))?;
        let control = layout
            .screens
            .iter()
            .find(|s| s.id.device == controller.0 && s.id.display == monitor.controller_display)
            .ok_or_else(|| invalid("The controller display is no longer connected"))?;
        let rect = Rect::new(x, 0.0, control.rect.w, control.rect.h);
        visible.push(PhysicalScreen {
            device: native.id.device.to_hex(),
            display: native.id.display.clone(),
            rect,
        });
        x += rect.w;
    }
    if !oink_display::layout::valid_physical_screens(&visible) {
        return Err(invalid("Invalid physical monitor arrangement"));
    }
    Ok(visible)
}

trait Ddc {
    fn read(&mut self, display: &str) -> Result<u16, String>;
    fn write(&mut self, display: &str, input: u16) -> Result<(), String>;
}
struct LocalDdc;
impl Ddc for LocalDdc {
    fn read(&mut self, display: &str) -> Result<u16, String> {
        #[cfg(windows)]
        {
            windows_ddc::read(display)
        }
        #[cfg(not(windows))]
        {
            BetterDisplay.read(display)
        }
    }
    fn write(&mut self, display: &str, value: u16) -> Result<(), String> {
        #[cfg(windows)]
        {
            windows_ddc::write(display, value)
        }
        #[cfg(not(windows))]
        {
            BetterDisplay.write(display, value)
        }
    }
}
#[cfg(not(windows))]
struct BetterDisplay;
#[cfg(not(windows))]
impl BetterDisplay {
    fn run(args: &[String]) -> Result<String, String> {
        if !cfg!(target_os = "macos") {
            return Err("The physical monitor controller currently requires macOS and BetterDisplay".into());
        }
        let mut child = Process::new("/Applications/BetterDisplay.app/Contents/MacOS/BetterDisplay")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "BetterDisplay is unavailable; install it and enable command-line integration".to_string())?;
        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if start.elapsed() < Duration::from_secs(5) => std::thread::sleep(Duration::from_millis(25)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("BetterDisplay command timed out".into());
                }
            }
        };
        if !status.success() {
            return Err("BetterDisplay rejected the DDC command".into());
        }
        let mut output = String::new();
        child
            .stdout
            .take()
            .ok_or("Missing BetterDisplay reply")?
            .take(65536)
            .read_to_string(&mut output)
            .map_err(|_| "Invalid BetterDisplay reply")?;
        Ok(output)
    }
}
#[cfg(not(windows))]
impl Ddc for BetterDisplay {
    fn read(&mut self, display: &str) -> Result<u16, String> {
        if !uuid(display) {
            return Err("Invalid display UUID".into());
        }
        Self::run(&[
            "get".into(),
            format!("-UUID={display}"),
            "-feature=ddc".into(),
            "-vcp=inputSelect".into(),
            "-value".into(),
        ])?
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|v| *v != 0)
        .ok_or(())
        .map_err(|_| "The monitor did not report its current input; routing was not changed".into())
    }
    fn write(&mut self, display: &str, input: u16) -> Result<(), String> {
        if !uuid(display) || input == 0 {
            return Err("Invalid display UUID or input value".into());
        }
        Self::run(&[
            "set".into(),
            format!("-UUID={display}"),
            "-feature=ddc".into(),
            "-vcp=inputSelect".into(),
            format!("-value={input}"),
        ])?;
        for _ in 0..5 {
            std::thread::sleep(Duration::from_millis(150));
            if self.read(display).is_ok_and(|actual| actual == input) {
                return Ok(());
            }
        }
        Err(format!("Input {input} was not confirmed by the monitor"))
    }
}

/// Never publish ownership after an unconfirmed DDC write. Any uncertain write is
/// included in rollback, and restoration failures remain explicit to the caller.
fn transaction(
    ddc: &mut impl Ddc,
    targets: &[(String, u16)],
    commit: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let previous = targets
        .iter()
        .map(|(id, _)| ddc.read(id).map(|input| (id.clone(), input)))
        .collect::<Result<Vec<_>, _>>()?;
    let mut attempted = 0;
    let outcome = (|| {
        for ((id, input), (_, old)) in targets.iter().zip(&previous) {
            attempted += 1;
            if input != old {
                ddc.write(id, *input)?;
            }
        }
        commit()
    })();
    if let Err(error) = outcome {
        let mut failures = Vec::new();
        for (id, input) in previous[..attempted].iter().rev() {
            if let Err(e) = ddc.write(id, *input) {
                failures.push(format!("{id}: {e}"));
            }
        }
        return Err(if failures.is_empty() {
            format!("{error}. Previous inputs restored")
        } else {
            format!("{error}. Input restoration needs attention: {}", failures.join("; "))
        });
    }
    Ok(())
}

pub fn execute(node: &Node, command: Command) -> ApiResult<serde_json::Value> {
    let _gate = GATE.lock().unwrap_or_else(|e| e.into_inner());
    let path = node.data_dir().join("physical-monitors.json");
    let setup: Setup = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| invalid("Physical monitor setup is unreadable"))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Setup::default(),
        Err(_) => return Err(invalid("Cannot read physical monitor setup")),
    };
    let group = node.link_config()?;
    let baseline = group.applied();
    let controller = node.overview()?.this_device.id;
    let state = |setup: &Setup| serde_json::json!({"setup":setup, "active":group.applied().layout.settings.physical_screens, "controller_available":cfg!(any(target_os="macos",windows))});
    let mut ddc = LocalDdc;
    match command {
        Command::Query => Ok(state(&setup)),
        Command::Save {
            expected_revision,
            monitors,
        } => {
            if expected_revision != setup.revision {
                return Err(invalid("Setup changed; refresh and retry"));
            }
            if !baseline.layout.settings.physical_screens.is_empty() {
                return Err(invalid("Return to ordinary layout before changing monitor bindings"));
            }
            validate(&monitors)?;
            for monitor in &monitors {
                if !monitor
                    .bindings
                    .iter()
                    .any(|b| b.screen.device == controller && b.screen.display == monitor.controller_display)
                {
                    return Err(invalid(
                        "Bind this controller's exact native display identity to each physical monitor",
                    ));
                }
            }
            let updated = Setup {
                revision: setup
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| invalid("Setup revision exhausted"))?,
                monitors,
            };
            let bytes = serde_json::to_vec_pretty(&updated).map_err(|e| invalid(e.to_string()))?;
            oink_foundation::storage::atomic_write(&path, &bytes)
                .map_err(|_| invalid("Cannot save physical monitor setup"))?;
            Ok(state(&updated))
        }
        Command::Read { controller_display } => {
            Ok(serde_json::json!({"input":ddc.read(&controller_display).map_err(invalid)?}))
        }
        Command::Test {
            controller_display,
            input,
        } => {
            let old = ddc.read(&controller_display).map_err(invalid)?;
            let result = ddc.write(&controller_display, input);
            if result.is_ok() {
                std::thread::sleep(Duration::from_secs(5));
            }
            let restore = ddc.write(&controller_display, old);
            if let Err(e) = restore {
                return Err(invalid(format!("Test ended; input restoration failed: {e}")));
            }
            result.map_err(invalid)?;
            Ok(serde_json::json!({"restored_input":old}))
        }
        Command::Apply {
            expected_revision,
            screens,
        } => {
            if expected_revision != setup.revision {
                return Err(invalid("Setup changed; refresh and retry"));
            }
            for member in &group.members().members {
                if !member.is_self && (!member.online || !group.peer_supports(member.device, CAP)) {
                    return Err(invalid(
                        "Connect every paired computer using the physical-monitor fork before switching",
                    ));
                }
            }
            if node.overview()?.focus_lock.is_some() {
                return Err(invalid("Unlock pointer focus before switching physical monitors"));
            }
            let visible = plan(&setup, &screens, &baseline.layout, controller)?;
            for screen in &screens {
                if !baseline.owners.get(&screen.device.0).is_none_or(|o| o.screen_shared) {
                    return Err(invalid("Enable screen sharing on every selected computer"));
                }
            }
            let targets: Vec<_> = setup
                .monitors
                .iter()
                .zip(&screens)
                .map(|(m, s)| {
                    (
                        m.controller_display.clone(),
                        m.bindings
                            .iter()
                            .find(|b| &b.screen == s)
                            .expect("validated binding")
                            .input,
                    )
                })
                .collect();
            transaction(&mut ddc, &targets, || {
                node.commit_manual_layout_exact(|layout| {
                    if *layout != *baseline.layout {
                        return Err(invalid("Desktop changed during the switch; retry"));
                    }
                    layout.settings.physical_screens = visible.clone();
                    Ok(())
                })
                .map_err(|e| e.message().to_string())
            })
            .map_err(invalid)?;
            // Drop any invisible old focus after the new layout has been applied.
            node.input_thread()?
                .send(crate::threads::input::InputCommand::MovePointerTo(screens[0].device.0));
            Ok(state(&setup))
        }
        Command::Clear => {
            node.commit_manual_layout(|layout| {
                layout.settings.physical_screens.clear();
                Ok(())
            })?;
            Ok(state(&setup))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Fake {
        values: std::collections::BTreeMap<String, u16>,
        writes: Vec<(String, u16)>,
        fail: Option<String>,
    }
    impl Ddc for Fake {
        fn read(&mut self, id: &str) -> Result<u16, String> {
            self.values.get(id).copied().ok_or("No DDC".into())
        }
        fn write(&mut self, id: &str, value: u16) -> Result<(), String> {
            self.writes.push((id.into(), value));
            self.values.insert(id.into(), value);
            if self.fail.as_deref() == Some(id) {
                self.fail = None;
                Err("Unconfirmed write".into())
            } else {
                Ok(())
            }
        }
    }
    #[test]
    fn failed_second_write_restores_both_and_never_commits() {
        let mut io = Fake {
            values: [("L".into(), 17), ("R".into(), 17)].into(),
            fail: Some("R".into()),
            ..Default::default()
        };
        // Fail at R, not at L.
        struct FailRight(Fake);
        impl Ddc for FailRight {
            fn read(&mut self, id: &str) -> Result<u16, String> {
                self.0.read(id)
            }
            fn write(&mut self, id: &str, v: u16) -> Result<(), String> {
                self.0.values.insert(id.into(), v);
                if id == "R" && v == 8 {
                    Err("Unconfirmed".into())
                } else {
                    Ok(())
                }
            }
        }
        let mut ddc = FailRight(std::mem::take(&mut io));
        assert!(
            transaction(&mut ddc, &[("L".into(), 15), ("R".into(), 8)], || panic!(
                "must not commit"
            ))
            .is_err()
        );
        assert_eq!(ddc.0.values["L"], 17);
        assert_eq!(ddc.0.values["R"], 17);
    }
    #[test]
    fn failed_layout_commit_restores_inputs() {
        let mut io = Fake {
            values: [("L".into(), 17)].into(),
            ..Default::default()
        };
        assert!(transaction(&mut io, &[("L".into(), 15)], || Err("Stale".into())).is_err());
        assert_eq!(io.values["L"], 17);
    }
    #[test]
    fn missing_ddc_aborts_before_any_write() {
        let mut io = Fake {
            values: [("L".into(), 17)].into(),
            ..Default::default()
        };
        assert!(
            transaction(&mut io, &[("L".into(), 15), ("R".into(), 8)], || panic!(
                "must not commit"
            ))
            .is_err()
        );
        assert!(io.writes.is_empty());
    }
}
