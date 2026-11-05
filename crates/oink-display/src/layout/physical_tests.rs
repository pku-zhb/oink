use super::*;

fn native(device: DeviceId, name: &str, x: f64, group: f64) -> ScreenPlacement {
    ScreenPlacement {
        id: ScreenId::new(device, name),
        name: name.into(),
        local: Rect::new(x, 0., 100., 100.),
        rect: Rect::new(group + x, 0., 100., 100.),
        size_source: SizeSource::Manual,
        primary: x == 0.,
        builtin: false,
    }
}
#[test]
fn all_four_ownership_combinations_have_two_visible_screens_and_bidirectional_seams() {
    let a = DeviceId([1; 32]);
    let b = DeviceId([2; 32]);
    for left in [a, b] {
        for right in [a, b] {
            let mut layout = Layout {
                screens: vec![
                    native(a, "L", 0., 0.),
                    native(a, "R", 100., 0.),
                    native(b, "L", 0., 400.),
                    native(b, "R", 100., 400.),
                ],
                ..Default::default()
            };
            let original = layout.screens.clone();
            layout.settings.physical_screens = vec![
                PhysicalScreen {
                    device: left.to_hex(),
                    display: "L".into(),
                    rect: Rect::new(0., 0., 100., 100.),
                },
                PhysicalScreen {
                    device: right.to_hex(),
                    display: "R".into(),
                    rect: Rect::new(100., 0., 100., 100.),
                },
            ];
            let topology = layout.topology(&Default::default());
            let l = ScreenId::new(left, "L");
            let r = ScreenId::new(right, "R");
            let forward = topology
                .step(
                    &ScreenPos {
                        screen: l.clone(),
                        local: Point::new(99., 50.),
                    },
                    (4., 0.),
                )
                .unwrap();
            assert_eq!(forward.to.screen, r);
            let backward = topology
                .step(
                    &ScreenPos {
                        screen: r.clone(),
                        local: Point::new(101., 50.),
                    },
                    (-4., 0.),
                )
                .unwrap();
            assert_eq!(backward.to.screen, l);
            assert_eq!(topology.screens().iter().filter(|s| s.enabled).count(), 2);
            assert_eq!(layout.view(&Default::default(), |_| true).screens.len(), 2);
            assert_eq!(
                layout.screens, original,
                "physical routing must not rewrite native coordinates"
            );
            for owner in [a, b] {
                assert_eq!(topology.source_requires_target(owner), owner != left && owner != right);
            }
            layout.settings.physical_screens.clear();
            assert_eq!(layout.view(&Default::default(), |_| true).screens.len(), 4);
        }
    }
}

#[test]
fn invalid_or_duplicate_physical_rectangles_are_rejected() {
    let a = PhysicalScreen {
        device: DeviceId([1; 32]).to_hex(),
        display: "L".into(),
        rect: Rect::new(0., 0., 100., 100.),
    };
    assert!(valid_physical_screens(std::slice::from_ref(&a)));
    assert!(!valid_physical_screens(&[a.clone(), a.clone()]));
    assert!(!valid_physical_screens(&[PhysicalScreen {
        rect: Rect::new(f64::NAN, 0., 1., 1.),
        ..a
    }]));
}
