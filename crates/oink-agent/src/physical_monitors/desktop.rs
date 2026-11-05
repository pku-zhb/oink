//! Native desktop adapters. Enumeration is read-only; every write is explicit.
use super::model::DesktopPlan;
use oink_foundation::display::DisplayInfo;
use serde::{Deserialize, Serialize};

#[cfg(windows)]
mod windows;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub displays: Vec<DisplayInfo>,
    pub native: serde_json::Value,
}
pub fn displays() -> Result<Vec<DisplayInfo>, String> {
    oink_display::platform_backend()
        .map_err(|e| e.to_string())?
        .displays()
        .map_err(|e| e.to_string())
}
pub fn capture() -> Result<Snapshot, String> {
    #[cfg(windows)]
    let native = windows::capture()?;
    #[cfg(not(windows))]
    let native = serde_json::Value::Null;
    Ok(Snapshot {
        displays: displays()?,
        native,
    })
}
pub fn available() -> bool {
    #[cfg(windows)]
    {
        return true;
    }
    #[cfg(target_os = "macos")]
    {
        return std::path::Path::new("/Applications/BetterDisplay.app/Contents/MacOS/BetterDisplay").is_file();
    }
    #[allow(unreachable_code)]
    false
}
pub fn preflight(snapshot: &Snapshot, plan: &DesktopPlan) -> Result<(), String> {
    plan.retained(&snapshot.displays.iter().map(|d| d.key.clone()).collect::<Vec<_>>())?;
    #[cfg(target_os = "macos")]
    for key in &plan.managed {
        let output = super::BetterDisplay::run(&[
            "get".into(),
            format!("-UUID={key}"),
            "-connected".into(),
            "-value".into(),
        ])?;
        if !matches!(
            output.trim().to_ascii_lowercase().as_str(),
            "on" | "off" | "true" | "false" | "1" | "0"
        ) {
            return Err("BetterDisplay cannot report connection state; check Pro and connection management, or use compatible mode".into());
        }
    }
    Ok(())
}

pub fn apply(snapshot: &Snapshot, plan: &DesktopPlan) -> Result<(), String> {
    let keys: Vec<_> = snapshot.displays.iter().map(|d| d.key.clone()).collect();
    let retained = plan.retained(&keys)?;
    #[cfg(windows)]
    {
        return windows::apply(&snapshot.native, &retained);
    }
    #[cfg(target_os = "macos")]
    {
        // Connect first. The selected main must exist before any old screen leaves.
        for key in &plan.visible {
            better(key, "connected", "on")?;
        }
        let primary = snapshot
            .displays
            .iter()
            .find(|d| d.is_primary && retained.contains(&d.key))
            .or_else(|| snapshot.displays.iter().find(|d| retained.contains(&d.key)))
            .ok_or("A visible main display is required")?;
        better(&primary.key, "main", "on")?;
        for key in &plan.managed {
            if !retained.contains(key) {
                better(key, "connected", "off")?;
            }
        }
        let actual = displays()?;
        if plan
            .managed
            .iter()
            .any(|key| actual.iter().any(|d| &d.key == key) != retained.contains(key))
            || !actual.iter().any(|d| d.key == primary.key && d.is_primary)
        {
            return Err("The system did not confirm display visibility and main display; restoring".into());
        }
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err("System display management is not available on this platform".into())
}
pub fn restore(snapshot: &Snapshot) -> Result<(), String> {
    #[cfg(windows)]
    {
        return windows::restore(&snapshot.native);
    }
    #[cfg(target_os = "macos")]
    {
        let current = displays()?;
        // Never send a gratuitous reconnect to a display already active.
        for display in &snapshot.displays {
            if !current.iter().any(|d| d.key == display.key) {
                better(&display.key, "connected", "on")?;
            }
        }
        if let Some(primary) = snapshot.displays.iter().find(|d| d.is_primary)
            && !displays()?.iter().any(|d| d.key == primary.key && d.is_primary)
        {
            better(&primary.key, "main", "on")?;
        }
        let backend = oink_display::platform_backend().map_err(|e| e.to_string())?;
        let native = backend.native_configuration().map_err(|e| e.to_string())?;
        let origins: Vec<_> = native
            .sources()
            .map(|d| {
                let old = snapshot.displays.iter().find(|old| old.key == d.key);
                oink_display::arrangement::NativeOrigin {
                    key: d.key.clone(),
                    origin: old.map_or(oink_foundation::Point::new(d.bounds.x, d.bounds.y), |s| {
                        oink_foundation::Point::new(s.bounds.x, s.bounds.y)
                    }),
                }
            })
            .collect();
        backend
            .apply_native_origins(&native, &origins)
            .map_err(|e| e.to_string())?;
        let actual = displays()?;
        if snapshot.displays.iter().any(|s| {
            !actual
                .iter()
                .any(|d| d.key == s.key && d.bounds == s.bounds && d.is_primary == s.is_primary)
        }) {
            return Err("Display configuration has not returned to its saved state".into());
        }
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err("System display restoration is not supported".into())
}
#[cfg(target_os = "macos")]
fn better(display: &str, feature: &str, value: &str) -> Result<(), String> {
    let output = super::BetterDisplay::run(&["set".into(), format!("-UUID={display}"), format!("-{feature}={value}")])?;
    if output.trim().eq_ignore_ascii_case("failed.") || output.trim().eq_ignore_ascii_case("failed") {
        return Err(format!(
            "BetterDisplay could not change {feature}; check connection management and Pro availability"
        ));
    }
    std::thread::sleep(std::time::Duration::from_millis(400));
    Ok(())
}
