//! One short-lived, UI-independent recovery helper for each unconfirmed change.
use super::{Ddc, LocalDdc, desktop};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub display: String,
    pub value: u16,
    pub readback: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub id: String,
    pub deadline_ms: u64,
    pub desktop: Option<desktop::Snapshot>,
    pub inputs: Vec<Input>,
    #[serde(default)]
    pub route: Vec<oink_display::layout::PhysicalScreen>,
    pub outcome: Option<Result<(), String>>,
    #[serde(default)]
    pub ready: bool,
    #[serde(default)]
    pub helper_pid: Option<u32>,
    #[serde(default)]
    pub owner_boot: Option<[u8; 16]>,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub previous_choices: Vec<crate::api::ScreenKey>,
    #[serde(default)]
    pub previous_mode: Option<super::model::Mode>,
    #[serde(default)]
    pub visible_displays: Vec<String>,
    #[serde(default)]
    pub prompt_displays: Vec<oink_foundation::display::DisplayInfo>,
    #[serde(default)]
    pub confirmation: Option<super::experience::Confirmation>,
}
pub(super) fn path(root: &Path) -> PathBuf {
    root.join("physical-monitors").join("recovery.json")
}
pub(super) fn guard(root: &Path) -> Result<std::fs::File, String> {
    let dir = root.join("physical-monitors");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(dir.join("recovery.lock")).map_err(|e| e.to_string())?;
    fs2::FileExt::lock_exclusive(&file).map_err(|e| e.to_string())?;
    Ok(file)
}
pub(super) fn read(path: &Path) -> Result<Option<Record>, String> {
    use std::io::Read;
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("Recovery record exceeds its size limit".into());
    }
    let r: Record = serde_json::from_slice(&bytes).map_err(|_| "Recovery record is unreadable")?;
    if r.id.len() != 32
        || !r.id.bytes().all(|b| b.is_ascii_hexdigit())
        || r.inputs.len() > 8
        || r.inputs
            .iter()
            .any(|i| i.value == 0 || i.display.is_empty() || i.display.len() > 512)
    {
        return Err("Invalid recovery record".into());
    }
    Ok(Some(r))
}
pub(super) fn write(path: &Path, record: &Record) -> Result<(), String> {
    let parent = path.parent().ok_or("Missing recovery directory")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    let bytes = serde_json::to_vec(record).map_err(|e| e.to_string())?;
    oink_foundation::storage::atomic_write(path, &bytes).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // explicitly disarmed after atomic publication
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}
pub(super) fn restore(record: &Record) -> Result<(), String> {
    let mut failures = Vec::new();
    if let Some(snapshot) = &record.desktop
        && let Err(e) = desktop::restore(snapshot)
    {
        failures.push(e);
    }
    let mut ddc = LocalDdc;
    for input in &record.inputs {
        if input.readback && ddc.read(&input.display) == Ok(input.value) {
            continue;
        }
        let result = if input.readback {
            ddc.write(&input.display, input.value)
        } else {
            ddc.write_only(&input.display, input.value)
        };
        if let Err(e) = result {
            failures.push(format!("{}: {e}", input.display));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}
pub(super) fn spawn(path: &Path) -> Result<(), String> {
    if read(path)?.is_some_and(|r| r.ready && r.helper_pid.is_some_and(alive)) {
        return Ok(());
    }
    let mut command = std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    command.env_clear();
    for key in [
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "LANG",
        "LC_CTYPE",
        "TMPDIR",
        "__CF_USER_TEXT_ENCODING",
        "SystemRoot",
        "WINDIR",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "TEMP",
        "TMP",
        "HOMEDRIVE",
        "HOMEPATH",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .arg("--physical-monitor-recovery")
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = crate::backend::launch::spawn_detached(&mut command).map_err(|e| e.to_string())?;
    let started = std::time::Instant::now();
    loop {
        if read(path)?.is_some_and(|r| r.ready && r.helper_pid == Some(child.id())) {
            break;
        }
        if child.try_wait().map_err(|e| e.to_string())?.is_some() || started.elapsed() > Duration::from_secs(3) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("The independent display recovery helper did not become ready".into());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
/// Called before either backend or CLI initialization. The helper does not
/// initialize networking, capture input, or inherit any application credentials.
pub fn dispatch() -> Option<std::process::ExitCode> {
    let mut args = std::env::args_os();
    args.next();
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--physical-monitor-recovery")) {
        return None;
    }
    let Some(path) = args.next().map(PathBuf::from) else {
        return Some(std::process::ExitCode::FAILURE);
    };
    let result = (|| {
        if !path.is_absolute()
            || path.file_name() != Some(std::ffi::OsStr::new("recovery.json"))
            || path.parent().and_then(Path::file_name) != Some(std::ffi::OsStr::new("physical-monitors"))
        {
            return Err("Invalid helper path".to_string());
        }
        let Some(mut first) = read(&path)? else { return Ok(()) };
        first.ready = true;
        first.helper_pid = Some(std::process::id());
        write(&path, &first)?;
        loop {
            let directory = path.parent().and_then(Path::parent).ok_or("Invalid recovery root")?;
            let file_guard = guard(directory)?;
            let Some(mut current) = read(&path)? else { return Ok(()) };
            if current.id != first.id || current.outcome.is_some() {
                return Ok(());
            }
            if now_ms() >= current.deadline_ms {
                current.outcome = Some(restore(&current));
                if let Some(prompt) = current.confirmation.as_mut() {
                    prompt.phase = if current.outcome.as_ref().is_some_and(Result::is_ok) {
                        "reverted"
                    } else {
                        "error"
                    }
                    .into();
                    prompt.deadline_ms = now_ms() + 30_000;
                }
                write(&path, &current)?;
                drop(file_guard);
                use crate::api::AgentApi;
                let _ = crate::remote::RemoteApi::new(directory).physical_monitors(super::Command::Experience {
                    request: super::experience::Request::Recovered { operation: current.id },
                });
                return Ok(());
            }
            drop(file_guard);
            std::thread::sleep(Duration::from_millis(200));
        }
    })();
    if let Err(error) = &result {
        eprintln!("Physical display recovery failed: {error}");
    }
    Some(if result.is_ok() {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    })
}

pub(super) fn alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: signal zero only observes process existence; it sends no signal.
        pid <= i32::MAX as u32 && unsafe { libc::kill(pid as i32, 0) } == 0
    }
    #[cfg(windows)]
    {
        use windows::Win32::{
            Foundation::CloseHandle,
            System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
        };
        // SAFETY: a query-only handle is closed after the synchronous status read.
        unsafe {
            let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return false;
            };
            let mut code = 0;
            let running = GetExitCodeProcess(handle, &mut code).is_ok() && code == 259;
            let _ = CloseHandle(handle);
            running
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn record_is_private_bounded_and_rejects_invalid_hardware_addresses() {
        let d = tempfile::tempdir().unwrap();
        let p = path(d.path());
        let record = Record {
            id: "a".repeat(32),
            deadline_ms: 1,
            desktop: None,
            inputs: vec![],
            route: vec![],
            outcome: None,
            ready: false,
            helper_pid: None,
            owner_boot: None,
            visible_displays: vec![],
            prompt_displays: vec![],
            confirmation: None,
            group: String::new(),
            previous_choices: vec![],
            previous_mode: None,
        };
        write(&p, &record).unwrap();
        assert_eq!(read(&p).unwrap().unwrap().id, record.id);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let mut invalid = record;
        invalid.inputs.push(Input {
            display: "x".into(),
            value: 0,
            readback: false,
        });
        write(&p, &invalid).unwrap();
        assert!(read(&p).is_err());
    }
}
