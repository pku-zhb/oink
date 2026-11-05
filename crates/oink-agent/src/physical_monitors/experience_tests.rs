//! Real authenticated peer transport and fake display hardware. Never touches OS displays.
use super::{desktop, experience::*, hardware::Platform};
use crate::{
    api::DeviceKey,
    node::{Node, NodeOptions, lock},
    snapshot_tests::{active, pair, until},
};
use oink_display::{
    ChangeHandler, DisplayBackend, DisplayError,
    arrangement::{NativeConfiguration, NativeDisplay},
};
use oink_foundation::{Rect, display::DisplayInfo};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

struct Hardware {
    displays: Mutex<Vec<DisplayInfo>>,
    original: Vec<DisplayInfo>,
    inputs: Mutex<BTreeMap<String, u16>>,
    callbacks: Mutex<Vec<ChangeHandler>>,
    fail_apply: AtomicBool,
}
use std::collections::BTreeMap;
impl Hardware {
    fn new(suffix: &str) -> Arc<Self> {
        let original: Vec<_> = ["left", "right"]
            .into_iter()
            .enumerate()
            .map(|(i, name)| DisplayInfo {
                key: format!("{name}-{suffix}"),
                name: name.into(),
                bounds: Rect::new(i as f64 * 100.0, 0.0, 100.0, 100.0),
                pixel_width: 100,
                pixel_height: 100,
                reference_extent: None,
                rotation_deg: 0,
                is_primary: i == 0,
                is_builtin: false,
                is_virtual: false,
            })
            .collect();
        Arc::new(Self {
            displays: Mutex::new(original.clone()),
            inputs: Mutex::new(original.iter().map(|d| (d.key.clone(), 17)).collect()),
            original,
            callbacks: Mutex::default(),
            fail_apply: AtomicBool::new(false),
        })
    }
    fn changed(&self) {
        for callback in lock(&self.callbacks).clone() {
            callback();
        }
    }
}
impl Platform for Hardware {
    fn capture(&self) -> Result<desktop::Snapshot, String> {
        Ok(desktop::Snapshot {
            displays: lock(&self.displays).clone(),
            native: serde_json::Value::Null,
        })
    }
    fn apply(&self, s: &desktop::Snapshot, p: &DesktopPlan) -> Result<(), String> {
        if self.fail_apply.load(Ordering::Relaxed) {
            return Err("injected native failure".into());
        }
        let retained = p.retained(&s.displays.iter().map(|d| d.key.clone()).collect::<Vec<_>>())?;
        let mut displays: Vec<_> = s
            .displays
            .iter()
            .filter(|d| retained.contains(&d.key))
            .cloned()
            .collect();
        let x = displays[0].bounds.x;
        for (i, d) in displays.iter_mut().enumerate() {
            d.bounds.x -= x;
            d.is_primary = i == 0;
        }
        *lock(&self.displays) = displays;
        self.changed();
        Ok(())
    }
    fn restore(&self, s: &desktop::Snapshot) -> Result<(), String> {
        *lock(&self.displays) = s.displays.clone();
        self.changed();
        Ok(())
    }
    fn read(&self, d: &str) -> Result<u16, String> {
        lock(&self.inputs).get(d).copied().ok_or("unknown input".into())
    }
    fn write(&self, d: &str, v: u16, _readback: bool) -> Result<(), String> {
        lock(&self.inputs).insert(d.into(), v);
        Ok(())
    }
    fn calibration_duration(&self) -> std::time::Duration {
        std::time::Duration::from_secs(5)
    }
}
impl DisplayBackend for Hardware {
    fn displays(&self) -> Result<Vec<DisplayInfo>, DisplayError> {
        Ok(lock(&self.displays).clone())
    }
    fn watch(&self, callback: ChangeHandler) -> Result<(), DisplayError> {
        lock(&self.callbacks).push(callback);
        Ok(())
    }
    fn native_configuration(&self) -> Result<NativeConfiguration, DisplayError> {
        Ok(NativeConfiguration {
            displays: lock(&self.displays)
                .iter()
                .map(|d| NativeDisplay {
                    key: d.key.clone(),
                    bounds: d.bounds,
                    primary: d.is_primary,
                    mirror_of: None,
                    mode: "fixture".into(),
                })
                .collect(),
        })
    }
}
async fn node(path: std::path::PathBuf, name: &str, hardware: Arc<Hardware>) -> Node {
    let n = Node::start(NodeOptions {
        net: oink_test_support::network_config(path, name, 0),
        input: Arc::new(oink_test_support::input::FakeInput::new()),
        displays: vec![],
        display_backend: Some(hardware.clone()),
        clipboard: None,
        mode_switch_in_callback: true,
        capture_reports_key_repeat: false,
        cursor_trail: false,
    })
    .await
    .unwrap();
    assert!(n.inner.physical.hardware.set(hardware).is_ok());
    n
}
async fn invoke(node: &Node, request: Request) -> Result<serde_json::Value, String> {
    let n = node.clone();
    tokio::task::spawn_blocking(move || execute(&n, request)).await.unwrap()
}
async fn await_phase(node: &Node, wanted: &str) {
    until("physical desktop operation reaches expected state", || {
        let o = lock(&node.inner.physical.operation);
        o.as_ref().is_some_and(|o| o.phase == wanted || !o.busy())
    })
    .await;
    let o = lock(&node.inner.physical.operation);
    let o = o.as_ref().unwrap();
    assert_eq!(o.phase, wanted, "{}", o.detail);
}
fn mapping(a: &Node, b: &Node) -> Mapping {
    Mapping {
        revision: 0,
        group: "initial".into(),
        coordinator: a.id().into(),
        panels: ["left", "right"]
            .into_iter()
            .map(|name| Panel {
                id: format!("panel-{name}"),
                name: name.into(),
                width: 1.0,
                height: 1.0,
                readback: false,
                baseline: if name == "right" { b.id().into() } else { a.id().into() },
                calibrated: true,
                endpoints: vec![
                    Endpoint {
                        screen: crate::api::ScreenKey {
                            device: a.id().into(),
                            display: format!("{name}-a"),
                        },
                        input: 17,
                        port: "HDMI 1".into(),
                    },
                    Endpoint {
                        screen: crate::api::ScreenKey {
                            device: b.id().into(),
                            display: format!("{name}-b"),
                        },
                        input: 18,
                        port: "HDMI 2".into(),
                    },
                ],
            })
            .collect(),
    }
}

#[test]
fn authenticated_desktop_transactions_and_recovery() {
    crate::test_watchdog::run(module_path!(), "physical-desktop-transactions", Some(35), async {
        let dir = tempfile::tempdir().unwrap();
        let ha = Hardware::new("a");
        let hb = Hardware::new("b");
        let a = node(dir.path().join("a"), "a", ha.clone()).await;
        let b = node(dir.path().join("b"), "b", hb.clone()).await;
        pair(&a, &b).await;
        active(&a, &b).await;
        lock(&ha.inputs).insert("right-a".into(), 18);
        lock(&hb.inputs).insert("right-b".into(), 18);
        let m = mapping(&a, &b);
        // A local storage failure must not strand a newer mapping on a peer.
        store(&a).unwrap();
        let blocked = a.data_dir().join("physical-monitors").join("mapping.json");
        std::fs::create_dir_all(&blocked).unwrap();
        assert!(
            invoke(
                &a,
                Request::Save {
                    expected_revision: 0,
                    mapping: m.clone()
                }
            )
            .await
            .is_err()
        );
        assert!(store(&b).unwrap().mapping.is_none());
        std::fs::remove_dir(&blocked).unwrap();
        invoke(
            &a,
            Request::Save {
                expected_revision: 0,
                mapping: m.clone(),
            },
        )
        .await
        .unwrap();
        assert_eq!(store(&a).unwrap().mapping, store(&b).unwrap().mapping);
        let saved = require_mapping(&a).unwrap();
        assert!(
            invoke(
                &a,
                Request::Save {
                    expected_revision: 0,
                    mapping: m
                }
            )
            .await
            .is_err()
        );
        let mixed = vec![
            saved.panels[0].endpoints[0].screen.clone(),
            saved.panels[1].endpoints[1].screen.clone(),
        ];
        invoke(
            &b,
            Request::Apply {
                expected_revision: 1,
                choices: mixed.clone(),
                mode: Mode::Complete,
                baseline_confirmed: true,
            },
        )
        .await
        .unwrap();
        await_phase(&a, "awaiting_confirmation").await;
        until("visible peer has armed its local confirmation", || {
            super::recovery::read(&super::recovery::path(b.data_dir()))
                .ok()
                .flatten()
                .is_some_and(|r| r.confirmation.is_some())
        })
        .await;
        let prompt = invoke(&b, Request::Confirmation).await.unwrap();
        assert_eq!(prompt["phase"], "pending");
        assert_eq!(prompt["bounds"]["x"], 0.0);
        assert_eq!(prompt["previous_bounds"]["x"], 100.0);

        assert_eq!(lock(&ha.displays).len(), 1);
        assert_eq!(lock(&hb.displays).len(), 1);
        assert!(lock(&hb.displays)[0].is_primary);
        assert_eq!(lock(&hb.displays)[0].bounds.x, 0.0);
        assert!(
            invoke(
                &b,
                Request::Apply {
                    expected_revision: 1,
                    choices: mixed.clone(),
                    mode: Mode::Complete,
                    baseline_confirmed: true
                }
            )
            .await
            .is_err()
        );
        let op = lock(&a.inner.physical.operation).clone().unwrap().id;
        assert!(
            invoke(
                &b,
                Request::Keep {
                    operation: "stale".into()
                }
            )
            .await
            .is_err()
        );
        invoke(&b, Request::Keep { operation: op }).await.unwrap();
        await_phase(&a, "completed").await;
        assert_eq!(store(&b).unwrap().active, mixed);
        // All restoration domains participate, including peer desktops and routing.
        invoke(&a, Request::Restore).await.unwrap();
        await_phase(&a, "reverted").await;
        assert_eq!(*lock(&ha.displays), ha.original);
        assert_eq!(*lock(&hb.displays), hb.original);
        assert!(
            a.link_config()
                .unwrap()
                .applied()
                .layout
                .settings
                .physical_screens
                .is_empty()
        );
        assert!(store(&b).unwrap().active.is_empty());

        // A failed native owner must roll back already changed inputs and owners.
        *lock(&a.inner.physical.cooldown) = 0;
        hb.fail_apply.store(true, Ordering::Relaxed);
        invoke(
            &a,
            Request::Apply {
                expected_revision: 1,
                choices: mixed.clone(),
                mode: Mode::Complete,
                baseline_confirmed: true,
            },
        )
        .await
        .unwrap();
        await_phase(&a, "reverted").await;
        assert_eq!(*lock(&ha.displays), ha.original);
        assert_eq!(*lock(&hb.displays), hb.original);
        assert_eq!(ha.read("right-a"), Ok(18));
        assert!(store(&a).unwrap().active.is_empty());
        hb.fail_apply.store(false, Ordering::Relaxed);
        invoke(&a, Request::Restore).await.unwrap();
        await_phase(&a, "reverted").await;

        // Unconfirmed changes automatically roll back, without a live UI.
        *lock(&a.inner.physical.cooldown) = 0;
        a.inner.physical.confirm_ms.store(5000, Ordering::Relaxed);
        invoke(
            &a,
            Request::Apply {
                expected_revision: 1,
                choices: mixed,
                mode: Mode::Complete,
                baseline_confirmed: true,
            },
        )
        .await
        .unwrap();
        await_phase(&a, "awaiting_confirmation").await;
        until("confirmation timeout restores both computers", || {
            lock(&a.inner.physical.operation)
                .as_ref()
                .is_some_and(|o| o.phase == "reverted")
        })
        .await;
        assert_eq!(*lock(&ha.displays), ha.original);
        assert_eq!(*lock(&hb.displays), hb.original);
        invoke(&a, Request::Restore).await.unwrap();
        await_phase(&a, "reverted").await;

        // Identical layouts that give one computer no visible screen remain
        // usable in explicit compatible mode, without disconnecting its last screen.
        *lock(&a.inner.physical.cooldown) = 0;
        a.inner.physical.confirm_ms.store(0, Ordering::Relaxed);
        let both = saved.panels.iter().map(|p| p.endpoints[1].screen.clone()).collect();
        invoke(
            &a,
            Request::Apply {
                expected_revision: 1,
                choices: both,
                mode: Mode::Compatible,
                baseline_confirmed: true,
            },
        )
        .await
        .unwrap();
        await_phase(&a, "awaiting_confirmation").await;
        until("headless coordinator has a visible peer prompt", || {
            super::recovery::read(&super::recovery::path(b.data_dir()))
                .ok()
                .flatten()
                .is_some_and(|r| r.confirmation.is_some())
        })
        .await;
        assert!(invoke(&a, Request::Confirmation).await.unwrap().is_null());
        assert_eq!(invoke(&b, Request::Confirmation).await.unwrap()["phase"], "pending");

        let op = lock(&a.inner.physical.operation).clone().unwrap().id;
        invoke(&b, Request::Revert { operation: op }).await.unwrap();
        await_phase(&a, "reverted").await;
        assert_eq!(*lock(&hb.displays), hb.original);
        invoke(&a, Request::Restore).await.unwrap();
        await_phase(&a, "reverted").await;

        // One-time calibration is separate from accepting hardware ownership.
        *lock(&a.inner.physical.cooldown) = 0;
        invoke(
            &a,
            Request::Calibrate {
                panel: saved.panels[0].clone(),
                target: b.id().into(),
            },
        )
        .await
        .unwrap();
        await_phase(&a, "calibrating").await;
        let op = lock(&a.inner.physical.operation).clone().unwrap().id;
        invoke(&a, Request::Revert { operation: op }).await.unwrap();
        await_phase(&a, "calibration_ready").await;
        assert_eq!(ha.read("left-a"), Ok(17));
        a.shutdown().await;
        b.shutdown().await;
    });
}

#[test]
fn exact_monitor_identity_and_calibration_are_required() {
    use oink_foundation::DeviceId;
    let device = DeviceKey(DeviceId([1; 32]));
    let endpoint = Endpoint {
        screen: crate::api::ScreenKey {
            device,
            display: "a".into(),
        },
        input: 17,
        port: "HDMI".into(),
    };
    let panel = Panel {
        id: "one".into(),
        name: "Screen".into(),
        width: 1.0,
        height: 1.0,
        endpoints: vec![endpoint.clone()],
        readback: false,
        baseline: device,
        calibrated: false,
    };
    let mut mapping = Mapping {
        revision: 1,
        group: "g".into(),
        coordinator: device,
        panels: vec![panel.clone()],
    };
    assert!(mapping.validate().is_ok());
    mapping.panels.push(Panel {
        id: "two".into(),
        ..panel
    });
    assert!(mapping.validate().is_err());
    mapping.panels.truncate(1);
    mapping.panels[0].width = f64::NAN;
    assert!(mapping.validate().is_err());
    mapping.panels[0].width = 1.0;
    assert!(
        mapping
            .rectangles(&[crate::api::ScreenKey {
                device,
                display: "unbound".into()
            }])
            .is_err()
    );
}
