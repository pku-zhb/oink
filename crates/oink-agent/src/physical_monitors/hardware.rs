//! Injectable hardware boundary: integration tests never contact real displays.
use super::{Ddc, LocalDdc, desktop, model::DesktopPlan, recovery::Record};
use std::time::Duration;
pub(super) trait Platform: Send + Sync {
    fn preflight(&self, _snapshot: &desktop::Snapshot, _plan: &DesktopPlan) -> Result<(), String> {
        Ok(())
    }
    fn capture(&self) -> Result<desktop::Snapshot, String>;
    fn apply(&self, snapshot: &desktop::Snapshot, plan: &DesktopPlan) -> Result<(), String>;
    fn restore(&self, snapshot: &desktop::Snapshot) -> Result<(), String>;
    fn read(&self, display: &str) -> Result<u16, String>;
    fn write(&self, display: &str, input: u16, readback: bool) -> Result<(), String>;
    fn calibration_duration(&self) -> Duration {
        Duration::from_secs(10)
    }
    fn restore_record(&self, record: &Record) -> Result<(), String> {
        let mut failures = Vec::new();
        if let Some(snapshot) = &record.desktop
            && let Err(e) = self.restore(snapshot)
        {
            failures.push(e);
        }
        for input in &record.inputs {
            if input.readback && self.read(&input.display) == Ok(input.value) {
                continue;
            }
            if let Err(e) = self.write(&input.display, input.value, input.readback) {
                failures.push(format!("{}: {e}", input.display));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; "))
        }
    }
}
pub(super) struct Real;
impl Platform for Real {
    fn preflight(&self, snapshot: &desktop::Snapshot, plan: &DesktopPlan) -> Result<(), String> {
        desktop::preflight(snapshot, plan)
    }
    fn capture(&self) -> Result<desktop::Snapshot, String> {
        desktop::capture()
    }
    fn apply(&self, snapshot: &desktop::Snapshot, plan: &DesktopPlan) -> Result<(), String> {
        desktop::apply(snapshot, plan)
    }
    fn restore(&self, snapshot: &desktop::Snapshot) -> Result<(), String> {
        if desktop::displays()? == snapshot.displays {
            return Ok(());
        }
        desktop::restore(snapshot)
    }
    fn read(&self, display: &str) -> Result<u16, String> {
        LocalDdc.read(display)
    }
    fn write(&self, display: &str, input: u16, readback: bool) -> Result<(), String> {
        LocalDdc.write_only(display, input)?;
        std::thread::sleep(Duration::from_secs(1));
        if readback {
            let started = std::time::Instant::now();
            for _ in 0..8 {
                if LocalDdc.read(display) == Ok(input) {
                    return Ok(());
                }
                if started.elapsed() > Duration::from_secs(6) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            return Err("The monitor did not confirm its new input; restoring".into());
        }
        Ok(())
    }
}
