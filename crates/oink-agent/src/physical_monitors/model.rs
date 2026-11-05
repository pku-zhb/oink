//! Hardware-independent physical desktop identity and transition planning.
use crate::api::{DeviceKey, ScreenKey};
use oink_display::layout::PhysicalScreen;
use oink_foundation::Rect;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub screen: ScreenKey,
    pub input: u16,
    #[serde(default)]
    pub port: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Panel {
    pub id: String,
    pub name: String,
    pub width: f64,
    pub height: f64,
    pub endpoints: Vec<Endpoint>,
    pub readback: bool,
    pub baseline: DeviceKey,
    /// A successful command alone is not a completed calibration.
    #[serde(default)]
    pub calibrated: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mapping {
    pub revision: u64,
    pub group: String,
    pub coordinator: DeviceKey,
    pub panels: Vec<Panel>,
}
impl Mapping {
    pub fn participants(&self) -> BTreeSet<DeviceKey> {
        self.panels
            .iter()
            .flat_map(|p| p.endpoints.iter().map(|e| e.screen.device))
            .collect()
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.group.is_empty() || self.group.len() > 64 || self.panels.is_empty() || self.panels.len() > 8 {
            return Err("Configure between one and eight physical monitors".into());
        }
        let (mut ids, mut screens) = (BTreeSet::new(), BTreeSet::new());
        for panel in &self.panels {
            if panel.id.is_empty()
                || panel.id.len() > 64
                || !ids.insert(&panel.id)
                || panel.name.trim().is_empty()
                || panel.name.len() > 128
                || ![panel.width, panel.height]
                    .into_iter()
                    .all(|n| n.is_finite() && n > 0.0 && n < 100_000.0)
                || panel.endpoints.is_empty()
                || panel.endpoints.len() > 8
            {
                return Err("Invalid physical monitor identity or size".into());
            }
            let (mut devices, mut inputs) = (BTreeSet::new(), BTreeSet::new());
            for endpoint in &panel.endpoints {
                if endpoint.input == 0
                    || endpoint.port.len() > 128
                    || endpoint.screen.display.is_empty()
                    || endpoint.screen.display.len() > 512
                    || !devices.insert(endpoint.screen.device)
                    || !inputs.insert(endpoint.input)
                    || !screens.insert(endpoint.screen.clone())
                {
                    return Err("Bind each system display once and one input per computer".into());
                }
            }
            if !devices.contains(&panel.baseline) || !devices.contains(&self.coordinator) {
                return Err("Each monitor needs a confirmed baseline and a coordinator connection".into());
            }
        }
        Ok(())
    }
    pub fn rectangles(&self, choices: &[ScreenKey]) -> Result<Vec<PhysicalScreen>, String> {
        self.validate()?;
        if choices.len() != self.panels.len() {
            return Err("Choose one computer for each physical monitor".into());
        }
        let mut x = 0.0;
        self.panels
            .iter()
            .zip(choices)
            .map(|(panel, key)| {
                if !panel.endpoints.iter().any(|e| &e.screen == key) {
                    return Err("The selected computer is not connected to this physical monitor".into());
                }
                let rectangle = PhysicalScreen {
                    device: key.device.0.to_hex(),
                    display: key.display.clone(),
                    rect: Rect::new(x, 0.0, panel.width, panel.height),
                };
                x += panel.width;
                Ok(rectangle)
            })
            .collect()
    }
    pub fn baseline_choices(&self) -> Vec<ScreenKey> {
        self.panels
            .iter()
            .filter_map(|p| {
                p.endpoints
                    .iter()
                    .find(|e| e.screen.device == p.baseline)
                    .map(|e| e.screen.clone())
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Complete,
    Compatible,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DesktopPlan {
    pub managed: Vec<String>,
    pub visible: Vec<String>,
}
impl DesktopPlan {
    pub fn for_device(mapping: &Mapping, choices: &[ScreenKey], device: DeviceKey) -> Self {
        Self {
            managed: mapping
                .panels
                .iter()
                .flat_map(|p| p.endpoints.iter())
                .filter(|e| e.screen.device == device)
                .map(|e| e.screen.display.clone())
                .collect(),
            visible: choices
                .iter()
                .filter(|e| e.device == device)
                .map(|e| e.display.clone())
                .collect(),
        }
    }
    /// Leave unrelated screens alone. Never disable the last physical connection.
    /// Headless input ownership remains usable via explicit compatible mode.
    pub fn retained(&self, connected: &[String]) -> Result<Vec<String>, String> {
        if self.managed.iter().collect::<BTreeSet<_>>().len() != self.managed.len()
            || self.visible.iter().any(|s| !self.managed.contains(s))
        {
            return Err("Invalid display ownership plan".into());
        }
        let retained: Vec<_> = connected
            .iter()
            .filter(|s| !self.managed.contains(s) || self.visible.contains(s))
            .cloned()
            .collect();
        if retained.is_empty() {
            return Err("This computer would have no active display; choose compatible mode".into());
        }
        if self.visible.iter().any(|s| !connected.contains(s)) {
            return Err("A configured display is disconnected; reconnect or recalibrate it".into());
        }
        Ok(retained)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_unmanaged_displays_and_refuses_unrecoverable_headless_topology() {
        let p = DesktopPlan {
            managed: vec!["left".into(), "right".into()],
            visible: vec!["left".into()],
        };
        assert_eq!(
            p.retained(&["left".into(), "right".into(), "internal".into()]).unwrap(),
            vec!["left", "internal"]
        );
        assert!(
            DesktopPlan { visible: vec![], ..p }
                .retained(&["left".into(), "right".into()])
                .is_err()
        );
    }
    #[test]
    fn stale_or_foreign_visible_displays_cannot_be_applied() {
        assert!(
            DesktopPlan {
                managed: vec!["a".into()],
                visible: vec!["b".into()]
            }
            .retained(&["a".into()])
            .is_err()
        );
        assert!(
            DesktopPlan {
                managed: vec!["a".into()],
                visible: vec!["a".into()]
            }
            .retained(&["b".into()])
            .is_err()
        );
    }
}
