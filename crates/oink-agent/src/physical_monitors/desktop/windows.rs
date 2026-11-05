//! Session-only CCD snapshots. Serialize initialized fields, never native padding.
use serde::{Deserialize, Serialize};
use windows::Win32::{
    Devices::Display::*,
    Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, LUID, POINTL},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Path {
    key: String,
    source_adapter: (u32, i32),
    source_id: u32,
    source_mode: u32,
    source_status: u32,
    target_adapter: (u32, i32),
    target_id: u32,
    target_mode: u32,
    target_status: u32,
    technology: i32,
    rotation: i32,
    scaling: i32,
    refresh: (u32, u32),
    scanline: i32,
    available: bool,
    flags: u32,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
enum Data {
    Source {
        width: u32,
        height: u32,
        format: i32,
        x: i32,
        y: i32,
    },
    Target {
        pixel_rate: u64,
        hsync: (u32, u32),
        vsync: (u32, u32),
        active: (u32, u32),
        total: (u32, u32),
        standard: u32,
        scanline: i32,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Mode {
    adapter: (u32, i32),
    id: u32,
    data: Data,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Saved {
    paths: Vec<Path>,
    modes: Vec<Mode>,
}
fn luid(v: (u32, i32)) -> LUID {
    LUID {
        LowPart: v.0,
        HighPart: v.1,
    }
}
fn pair(v: LUID) -> (u32, i32) {
    (v.LowPart, v.HighPart)
}
fn rational(v: (u32, u32)) -> DISPLAYCONFIG_RATIONAL {
    DISPLAYCONFIG_RATIONAL {
        Numerator: v.0,
        Denominator: v.1,
    }
}
fn ratio(v: DISPLAYCONFIG_RATIONAL) -> (u32, u32) {
    (v.Numerator, v.Denominator)
}
fn key(path: &DISPLAYCONFIG_PATH_INFO) -> Result<String, String> {
    let mut name = DISPLAYCONFIG_TARGET_DEVICE_NAME {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
            size: size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
            adapterId: path.targetInfo.adapterId,
            id: path.targetInfo.id,
        },
        ..Default::default()
    };
    // SAFETY: the header accurately describes the enclosing, initialized buffer.
    let result = unsafe { DisplayConfigGetDeviceInfo(&mut name.header) };
    if result != 0 {
        return Err(format!("Display target identity is unavailable: {result}"));
    }
    Ok(String::from_utf16_lossy(
        &name.monitorDevicePath[..name
            .monitorDevicePath
            .iter()
            .position(|v| *v == 0)
            .unwrap_or(name.monitorDevicePath.len())],
    ))
}
fn read() -> Result<Saved, String> {
    for _ in 0..4 {
        let (mut np, mut nm) = (0, 0);
        // SAFETY: live output counts; no caller-supplied memory is passed.
        let status = unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) };
        if status != ERROR_SUCCESS {
            return Err(format!("Cannot read the console desktop: {}", status.0));
        }
        if np > 256 || nm > 1024 {
            return Err("Display snapshot exceeds limits".into());
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        // SAFETY: arrays have the exact capacities reported above; counts remain live.
        let status = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut np,
                paths.as_mut_ptr(),
                &mut nm,
                modes.as_mut_ptr(),
                None,
            )
        };
        if status == ERROR_INSUFFICIENT_BUFFER {
            continue;
        }
        if status != ERROR_SUCCESS {
            return Err(format!("Cannot query the console desktop: {}", status.0));
        }
        paths.truncate(np as usize);
        modes.truncate(nm as usize);
        let paths = paths
            .iter()
            .map(|p| {
                // SAFETY: classic, non-virtual-aware query uses the full modeInfoIdx words.
                let (sm, tm) = unsafe { (p.sourceInfo.Anonymous.modeInfoIdx, p.targetInfo.Anonymous.modeInfoIdx) };
                Ok(Path {
                    key: key(p)?,
                    source_adapter: pair(p.sourceInfo.adapterId),
                    source_id: p.sourceInfo.id,
                    source_mode: sm,
                    source_status: p.sourceInfo.statusFlags,
                    target_adapter: pair(p.targetInfo.adapterId),
                    target_id: p.targetInfo.id,
                    target_mode: tm,
                    target_status: p.targetInfo.statusFlags,
                    technology: p.targetInfo.outputTechnology.0,
                    rotation: p.targetInfo.rotation.0,
                    scaling: p.targetInfo.scaling.0,
                    refresh: ratio(p.targetInfo.refreshRate),
                    scanline: p.targetInfo.scanLineOrdering.0,
                    available: p.targetInfo.targetAvailable.as_bool(),
                    flags: p.flags,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let modes = modes
            .iter()
            .map(|m| {
                let data = if m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE {
                    // SAFETY: the mode union tag was checked.
                    let s = unsafe { m.Anonymous.sourceMode };
                    Data::Source {
                        width: s.width,
                        height: s.height,
                        format: s.pixelFormat.0,
                        x: s.position.x,
                        y: s.position.y,
                    }
                } else if m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_TARGET {
                    // SAFETY: the mode union tag was checked. The signal union is a u32 word.
                    let s = unsafe { m.Anonymous.targetMode.targetVideoSignalInfo };
                    Data::Target {
                        pixel_rate: s.pixelRate,
                        hsync: ratio(s.hSyncFreq),
                        vsync: ratio(s.vSyncFreq),
                        active: (s.activeSize.cx, s.activeSize.cy),
                        total: (s.totalSize.cx, s.totalSize.cy),
                        standard: unsafe { s.Anonymous.videoStandard },
                        scanline: s.scanLineOrdering.0,
                    }
                } else {
                    return Err("Unsupported CCD mode; use compatible mode".into());
                };
                Ok(Mode {
                    adapter: pair(m.adapterId),
                    id: m.id,
                    data,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        return Ok(Saved { paths, modes });
    }
    Err("Display configuration did not settle".into())
}
pub(super) fn capture() -> Result<serde_json::Value, String> {
    serde_json::to_value(read()?).map_err(|e| e.to_string())
}
fn decode(value: &serde_json::Value) -> Result<Saved, String> {
    let s: Saved = serde_json::from_value(value.clone()).map_err(|_| "Invalid saved Windows desktop")?;
    if s.paths.is_empty()
        || s.paths.len() > 256
        || s.modes.len() > 1024
        || s.paths
            .iter()
            .any(|p| p.source_mode as usize >= s.modes.len() || p.target_mode as usize >= s.modes.len())
    {
        return Err("Invalid saved Windows display paths".into());
    }
    Ok(s)
}
fn write(saved: &Saved) -> Result<(), String> {
    let paths: Vec<_> = saved
        .paths
        .iter()
        .map(|p| DISPLAYCONFIG_PATH_INFO {
            sourceInfo: DISPLAYCONFIG_PATH_SOURCE_INFO {
                adapterId: luid(p.source_adapter),
                id: p.source_id,
                Anonymous: DISPLAYCONFIG_PATH_SOURCE_INFO_0 {
                    modeInfoIdx: p.source_mode,
                },
                statusFlags: p.source_status,
            },
            targetInfo: DISPLAYCONFIG_PATH_TARGET_INFO {
                adapterId: luid(p.target_adapter),
                id: p.target_id,
                Anonymous: DISPLAYCONFIG_PATH_TARGET_INFO_0 {
                    modeInfoIdx: p.target_mode,
                },
                outputTechnology: DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY(p.technology),
                rotation: DISPLAYCONFIG_ROTATION(p.rotation),
                scaling: DISPLAYCONFIG_SCALING(p.scaling),
                refreshRate: rational(p.refresh),
                scanLineOrdering: DISPLAYCONFIG_SCANLINE_ORDERING(p.scanline),
                targetAvailable: p.available.into(),
                statusFlags: p.target_status,
            },
            flags: p.flags,
        })
        .collect();
    let modes: Vec<_> = saved
        .modes
        .iter()
        .map(|m| {
            let mut result = DISPLAYCONFIG_MODE_INFO {
                adapterId: luid(m.adapter),
                id: m.id,
                ..Default::default()
            };
            match m.data {
                Data::Source {
                    width,
                    height,
                    format,
                    x,
                    y,
                } => {
                    result.infoType = DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE;
                    result.Anonymous.sourceMode = DISPLAYCONFIG_SOURCE_MODE {
                        width,
                        height,
                        pixelFormat: DISPLAYCONFIG_PIXELFORMAT(format),
                        position: POINTL { x, y },
                    };
                }
                Data::Target {
                    pixel_rate,
                    hsync,
                    vsync,
                    active,
                    total,
                    standard,
                    scanline,
                } => {
                    result.infoType = DISPLAYCONFIG_MODE_INFO_TYPE_TARGET;
                    result.Anonymous.targetMode = DISPLAYCONFIG_TARGET_MODE {
                        targetVideoSignalInfo: DISPLAYCONFIG_VIDEO_SIGNAL_INFO {
                            pixelRate: pixel_rate,
                            hSyncFreq: rational(hsync),
                            vSyncFreq: rational(vsync),
                            activeSize: DISPLAYCONFIG_2DREGION {
                                cx: active.0,
                                cy: active.1,
                            },
                            totalSize: DISPLAYCONFIG_2DREGION {
                                cx: total.0,
                                cy: total.1,
                            },
                            Anonymous: DISPLAYCONFIG_VIDEO_SIGNAL_INFO_0 {
                                videoStandard: standard,
                            },
                            scanLineOrdering: DISPLAYCONFIG_SCANLINE_ORDERING(scanline),
                        },
                    };
                }
            }
            result
        })
        .collect();
    // No SAVE_TO_DATABASE and no ALLOW_CHANGES: do not persist changes or silently
    // select a different refresh rate, color format or resolution.
    for flag in [SDC_VALIDATE, SDC_APPLY] {
        // SAFETY: typed paths and modes were rebuilt from the same bounded snapshot.
        let status = unsafe {
            SetDisplayConfig(
                Some(&paths),
                Some(&modes),
                SET_DISPLAY_CONFIG_FLAGS(flag.0 | SDC_USE_SUPPLIED_DISPLAY_CONFIG.0),
            )
        };
        if status != 0 {
            return Err(format!("Windows refused the display topology: {status}"));
        }
    }
    Ok(())
}
pub(super) fn restore(value: &serde_json::Value) -> Result<(), String> {
    let saved = decode(value)?;
    write(&saved)?;
    let current = read()?;
    if saved
        .paths
        .iter()
        .any(|p| !current.paths.iter().any(|a| a.key == p.key))
        || current.paths.len() != saved.paths.len()
    {
        return Err("Windows did not restore all original display paths".into());
    }
    Ok(())
}
pub(super) fn apply(value: &serde_json::Value, retained: &[String]) -> Result<(), String> {
    let mut saved = decode(value)?;
    let primary = saved
        .paths
        .iter()
        .find(|p| {
            retained.contains(&p.key)
                && matches!(
                    saved.modes[p.source_mode as usize].data,
                    Data::Source { x: 0, y: 0, .. }
                )
        })
        .or_else(|| saved.paths.iter().find(|p| retained.contains(&p.key)))
        .ok_or("No visible Windows main display")?;
    let Data::Source { x: dx, y: dy, .. } = saved.modes[primary.source_mode as usize].data else {
        return Err("Invalid source mode".into());
    };
    for mode in &mut saved.modes {
        if let Data::Source { x, y, .. } = &mut mode.data {
            *x = x.checked_sub(dx).ok_or("Display origin overflow")?;
            *y = y.checked_sub(dy).ok_or("Display origin overflow")?;
        }
    }
    saved.paths.retain(|p| retained.contains(&p.key));
    write(&saved)?;
    let current = read()?;
    if current.paths.len() != retained.len() || retained.iter().any(|key| !current.paths.iter().any(|p| &p.key == key))
    {
        return Err("Windows did not confirm the requested display paths".into());
    }
    Ok(())
}
