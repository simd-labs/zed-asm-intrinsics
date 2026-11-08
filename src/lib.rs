use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
};
use zed_extension_api::{
    self as zed, process::Command, set_language_server_installation_status as set_status,
    Architecture, DownloadedFileType, LanguageServerId, LanguageServerInstallationStatus as Status,
    Os, Result,
};

const ERR_HINT: &str = "simdref not found. Install it: uv tool install simdref (or pip install simdref), then run isa update. Instructions: https://github.com/simd-labs/simdref";

/// Pinned uv release. Bump UV_TAG to move to a newer uv.
const UV_TAG: &str = "0.12.23";

/// Pinned sha256 of each uv 0.12.23 release asset, lowercase hex. Change a
/// value with UV_TAG, from https://github.com/astral-sh/uv/releases/download/0.12.23/<asset>.sha256.
/// i686 on macOS has no uv release asset, so it has no pin.
fn uv_sha256(os: Os, arch: Architecture) -> Option<&'static str> {
    let (os, arch) = (
        match os {
            Os::Mac => "apple-darwin",
            Os::Linux => "unknown-linux-gnu",
            Os::Windows => "pc-windows-msvc",
        },
        match arch {
            Architecture::Aarch64 => "aarch64",
            Architecture::X86 => "i686",
            Architecture::X8664 => "x86_64",
        },
    );
    match (os, arch) {
        ("apple-darwin", "aarch64") => {
            Some("50487ae565ccd96e499056b4674d438f4c53170202617b4c759defe0c6a1b544")
        }
        ("apple-darwin", "x86_64") => {
            Some("960da44cb4b73685206ddd250b19e0a117fa41095710c1038f081f5cb613efb4")
        }
        ("unknown-linux-gnu", "aarch64") => {
            Some("6524bd338177ed50d035d39354e12545e993bbeba2ecbddf0480c5b3a81d313f")
        }
        ("unknown-linux-gnu", "i686") => {
            Some("52c93c40f6056c4d0e1e259acd60c11f7328416a4f28c85e5f1b04430538aeaa")
        }
        ("unknown-linux-gnu", "x86_64") => {
            Some("9167d72b3319674b6303c4cbe071854bba13ebdf3d76b1a7cbdc175471fb66d6")
        }
        ("pc-windows-msvc", "aarch64") => {
            Some("13294e232ececbe709c06b74e6ced06f2a225ea5591476685362f22be56a50d5")
        }
        ("pc-windows-msvc", "i686") => {
            Some("22a92f4374e4716c2848acaa0392f2f8e4a13b0ff65d080e2a5ade167bce96a8")
        }
        ("pc-windows-msvc", "x86_64") => {
            Some("75d05de6762778c31ee183398de7dd15093fad0ed90b1f236d8205ea5ec00c90")
        }
        ("apple-darwin", "i686") => None,
        _ => unreachable!(),
    }
}

struct AsmIntrinsics;

/// True when a regular file or a symlink exists at `path`. The WASI sandbox
/// refuses to follow uv's absolute tool shim symlinks outside their parent
/// preopen (`metadata` returns EPERM), so use `symlink_metadata`. Zed runs the
/// binary natively where the link resolves fine.
fn is_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_file() || m.file_type().is_symlink())
}

/// Returns the first file named `name` in a directory of the PATH string
/// `path_var`. On Windows the file name has the .exe suffix.
fn find_on_path(path_var: &str, name: &str) -> Option<PathBuf> {
    let name = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    path_var
        .split(if cfg!(windows) { ';' } else { ':' })
        .filter(|dir| !dir.is_empty())
        .filter_map(|dir| {
            let p = Path::new(dir).join(&name);
            is_file(&p).then_some(p)
        })
        .next()
}

/// Returns the system tar path. When no PATH directory holds it, the error
/// names tar, so the user sees the missing binary before the spawn.
fn tar_on_path() -> Result<PathBuf> {
    let path_var = env::var("PATH").unwrap_or_default();
    find_on_path(&path_var, "tar").ok_or_else(|| "tar not found on PATH".to_string())
}

/// Release asset stem and archive suffix of uv for a platform.
fn uv_asset(os: Os, arch: Architecture) -> (String, &'static str) {
    let arch = match arch {
        Architecture::Aarch64 => "aarch64",
        Architecture::X86 => "i686",
        Architecture::X8664 => "x86_64",
    };
    let (os, suffix) = match os {
        Os::Mac => ("apple-darwin", "tar.gz"),
        Os::Linux => ("unknown-linux-gnu", "tar.gz"),
        Os::Windows => ("pc-windows-msvc", "zip"),
    };
    (format!("uv-{arch}-{os}"), suffix)
}

fn run(program: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<()> {
    let out = Command::new(program.to_string_lossy())
        .args(args.iter().copied())
        .envs(env.iter().copied())
        .output()?;
    // Keep a full record of each command in install.log inside the work dir.
    // It is the first place a user looks when an install fails.
    let log = env::current_dir()
        .map_err(|e| e.to_string())?
        .join("install.log");
    let record = format!(
        "cmd: {} {}\nenv: {:?}\nstatus: {:?}\nstdout: {}\nstderr: {}\n---\n",
        program.display(),
        args.join(" "),
        env,
        out.status,
        String::from_utf8_lossy(&out.stdout).trim(),
        String::from_utf8_lossy(&out.stderr).trim()
    );
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .and_then(|mut f| std::io::Write::write_all(&mut f, record.as_bytes()))
        .ok();
    if out.status == Some(0) {
        return Ok(());
    }
    Err(format!(
        "{} {} exited with {:?}: {}",
        program.display(),
        args.join(" "),
        out.status,
        String::from_utf8_lossy(&out.stderr).trim()
    ))
}

/// Returns the path of the uv binary of the extracted archive in `work/uv`.
/// The tar.gz holds a top-level `uv-TRIPLE/` directory. The zip is flat.
fn uv_path(os: Os, stem: &str, work: &Path) -> PathBuf {
    let dir = work.join("uv");
    match os {
        Os::Windows => dir.join("uv.exe"),
        _ => dir.join(stem).join("uv"),
    }
}

/// Returns the uv binary in the work dir. It downloads uv when it is missing.
fn ensure_uv(id: &LanguageServerId, work: &Path) -> Result<PathBuf> {
    let (os, arch) = zed::current_platform();
    let (stem, suffix) = uv_asset(os, arch);
    let uv = uv_path(os, &stem, work);
    if is_file(&uv) {
        return Ok(uv);
    }
    set_status(id, &Status::CheckingForUpdate);
    let name = format!("{stem}.{suffix}");
    let url =
        format!("https://github.com/astral-sh/uv/releases/download/{UV_TAG}/{name}");
    // The code checks the archive sha256 against the pin in code, not the
    // release sidecar.
    let want = match uv_sha256(os, arch) {
        Some(h) => h,
        None => return Err(format!("no pinned uv sha256 for {name}")),
    };
    set_status(id, &Status::Downloading);
    // Record the pinned download in install.log. zed::download_file is a host
    // call, so `run` does not see it.
    let record = format!("download: {url}\nsha256: {want}\n---\n");
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(work.join("install.log"))
        .and_then(|mut f| std::io::Write::write_all(&mut f, record.as_bytes()))
        .ok();
    let result = download_extract(&url, &name, work, want)
        .map_err(|cause| format!("failed to download {name}: {cause}"));
    if result.is_err() {
        fs::remove_file(work.join(&name)).ok();
        fs::remove_dir_all(work.join("uv")).ok();
    }
    result?;
    if is_file(&uv) {
        Ok(uv)
    } else {
        Err(format!("{} missing after download", uv.display()))
    }
}

/// Downloads `url` and checks its SHA-256 against the pinned value. Then the
/// system `tar` opens the archive into `work/uv`.
fn download_extract(url: &str, name: &str, work: &Path, want: &str) -> Result<()> {
    zed::download_file(url, name, DownloadedFileType::Uncompressed)?;
    let bytes = fs::read(work.join(name)).map_err(|e| format!("cannot read {name}: {e}"))?;
    compare(&hex(&Sha256::digest(&bytes)), want)?;
    fs::create_dir_all(work.join("uv")).map_err(|e| e.to_string())?;
    // Windows 10+ ships bsdtar, which also reads .zip archives. Zed runs
    // process:exec outside the work dir, so both paths are absolute and no
    // -P is needed. Dropping -P keeps tar from honoring absolute member paths.
    // The capability args must match this call exactly: ["-xf", ARCHIVE, "-C", DIR].
    let dir = work.join("uv");
    let archive = work.join(name);
    let tar = tar_on_path()?;
    run(
        &tar,
        &["-xf", &*archive.to_string_lossy(), "-C", &*dir.to_string_lossy()],
        &[],
    )?;
    fs::remove_file(work.join(name)).ok();
    Ok(())
}

/// Errors with 'checksum mismatch' when the two lowercase hex digests differ.
fn compare(got: &str, want: &str) -> Result<()> {
    if got.len() == 64 && got == want {
        Ok(())
    } else {
        Err("checksum mismatch".to_string())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Returns the environment for every command the extension runs and for the
/// server it installed. It keeps every uv and simdref write inside the work
/// dir: simdref keeps its catalog in the XDG data dir, in `~/Library` on macOS
/// and in `%LOCALAPPDATA%` on Windows.
fn work_env(work: &Path) -> Vec<(String, String)> {
    let home = work.join("home");
    [
        ("UV_TOOL_DIR", work.join("tools")),
        ("UV_TOOL_BIN_DIR", work.join("bin")),
        ("UV_PYTHON_INSTALL_DIR", work.join("python")),
        ("UV_CACHE_DIR", work.join("uv-cache")),
        ("HOME", home.clone()),
        ("XDG_CONFIG_HOME", home.join(".config")),
        ("XDG_DATA_HOME", home.join(".local/share")),
        ("XDG_STATE_HOME", home.join(".local/state")),
        ("XDG_CACHE_HOME", home.join(".cache")),
        ("LOCALAPPDATA", home.join("AppData/Local")),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string_lossy().into_owned()))
    // A Python that uv downloads gets no Windows registry entry and no
    // `python` link in a bin dir outside the work dir.
    .chain([("UV_PYTHON_INSTALL_REGISTRY", "0"), ("UV_PYTHON_INSTALL_BIN", "0")]
        .map(|(k, v)| (k.to_string(), v.to_string())))
    .collect()
}

/// Returns the simdref-lsp path. It installs simdref into the work dir when needed.
fn install(id: &LanguageServerId) -> Result<PathBuf> {
    let work = env::current_dir().map_err(|e| e.to_string())?;
    let windows = matches!(zed::current_platform().0, Os::Windows);
    let exe = |name: &str| if windows { format!("{name}.exe") } else { name.to_string() };
    let (tools, bin) = (work.join("tools"), work.join("bin"));
    let lsp = bin.join(exe("simdref-lsp"));
    if is_file(&lsp) {
        return Ok(lsp);
    }
    let uv = ensure_uv(id, &work)?;
    set_status(id, &Status::Downloading);
    let env_pairs = work_env(&work);
    let env: Vec<(&str, &str)> = env_pairs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let isa = bin.join(exe("isa"));
    let result = run(&uv, &["tool", "install", "simdref"], &env)
        .and_then(|()| {
            if !is_file(&isa) {
                return Err(format!("{} missing after install", isa.display()));
            }
            run(&isa, &["update"], &env)
        })
        .and_then(|()| {
            if is_file(&lsp) {
                Ok(())
            } else {
                Err(format!("{} missing after install", lsp.display()))
            }
        });
    if result.is_err() {
        // A half install must not pass the is_file check on the next start.
        fs::remove_dir_all(&tools).ok();
        fs::remove_dir_all(&bin).ok();
    }
    result.map(|()| lsp)
}

/// Upgrades the simdref copy the extension installed in its work dir. A marker
/// file per UTC day throttles the check. The run then ends with
/// `isa vaddps --short`; it costs 0.4 s and 0 bytes when the catalog is
/// current. Every error is ignored: an older simdref is better than none.
fn upgrade() -> Option<()> {
    let work = env::current_dir().map_err(|e| e.to_string()).ok()?;
    let (os, arch) = zed::current_platform();
    let exe = |name: &str| if matches!(os, Os::Windows) { format!("{name}.exe") } else { name.to_string() };
    let isa = work.join("bin").join(exe("isa"));
    let uv = uv_path(os, &uv_asset(os, arch).0, &work);
    upgrade_with(&work, &isa, &uv, |p, a, e| {
        run(p, a, e).ok()?;
        Some(())
    })
}

/// Runs the daily upgrade sequence through a run closure.
fn upgrade_with<R>(work: &Path, isa: &Path, uv: &Path, run: R) -> Option<()>
where
    R: Fn(&Path, &[&str], &[(&str, &str)]) -> Option<()>,
{
    if !is_file(isa) || !is_file(uv) {
        return None;
    }
    // shortcut: a check still running at midnight UTC can overlap the next
    // day's check, add an OS lock if that is ever reported.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let marker = work.join(format!("update-{}", secs / 86_400));
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&marker)
        .ok()?;
    // Stale markers of older days go away. Ignore errors: the check must run.
    if let Ok(entries) = fs::read_dir(work) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("update-") && entry.path() != marker {
                fs::remove_file(entry.path()).ok();
            }
        }
    }
    let mut env_pairs = work_env(work);
    env_pairs.push(("UV_HTTP_TIMEOUT".to_string(), "120".to_string()));
    let env: Vec<(&str, &str)> = env_pairs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    // zed::Command has no timeout; a hung upgrade blocks the language-server start.
    // UV_HTTP_TIMEOUT bounds each uv HTTP read at 120 s. simdref's httpx
    // download uses a 120 s timeout per asset.
    let upgrade = run(uv, &["tool", "upgrade", "simdref"], &env);
    // Always refresh the catalog, even when the upgrade failed; the existing
    // simdref still works and `vaddps --short` costs 0.4 s when current.
    run(isa, &["vaddps", "--short"], &env);
    upgrade
}

impl zed::Extension for AsmIntrinsics {
    fn new() -> Self {
        AsmIntrinsics
    }

    fn language_server_command(
        &mut self,
        id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        // Only the copy the extension installed gets upgraded; a PATH install
        // stays as it is. The check runs before the server start command
        // returns, so it delays the start; the per-day marker file throttles
        // the delay to one network check a day.
        let path_install = worktree.which("simdref-lsp");
        if path_install.is_none() {
            upgrade();
        }
        // The installed server gets the work_env of its install, so it reads
        // the catalog that `isa update` wrote into the work dir.
        let (command, server_env) = match path_install {
            Some(path) => (path, vec![]),
            None => match install(id) {
                Ok(path) => {
                    set_status(id, &Status::None);
                    let work = env::current_dir().map_err(|e| e.to_string())?;
                    (path.to_string_lossy().into_owned(), work_env(&work))
                }
                Err(e) => {
                    let msg = format!("{ERR_HINT} Cause: {e}");
                    set_status(id, &Status::Failed(msg.clone()));
                    return Err(msg);
                }
            },
        };
        Ok(zed::Command {
            command,
            args: vec![],
            env: server_env,
        })
    }
}

zed::register_extension!(AsmIntrinsics);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_file_accepts_a_symlink_shim() {
        // uv tool shim is a symlink on Unix. The extension runs under WASI,
        // which refuses to follow absolute links (EPERM). is_file must still
        // find the shim so the install check does not delete a good install.
        let dir = env::temp_dir().join(format!("simdref-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("target");
        fs::write(&target, b"#!/bin/sh\n").unwrap();
        let link = dir.join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&target, &link).unwrap();
        assert!(is_file(&target));
        assert!(is_file(&link));
        assert!(!is_file(&dir.join("missing")));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn uv_asset_names_match_the_release() {
        let (stem, suffix) = uv_asset(Os::Linux, Architecture::X8664);
        assert_eq!(format!("{stem}.{suffix}"), "uv-x86_64-unknown-linux-gnu.tar.gz");
        let (stem, suffix) = uv_asset(Os::Mac, Architecture::Aarch64);
        assert_eq!(format!("{stem}.{suffix}"), "uv-aarch64-apple-darwin.tar.gz");
        let (stem, suffix) = uv_asset(Os::Windows, Architecture::X8664);
        assert_eq!(format!("{stem}.{suffix}"), "uv-x86_64-pc-windows-msvc.zip");
    }

    #[test]
    fn uv_sha256_pins_every_platform_with_a_uv_release_asset() {
        // uv 0.12.23 ships an asset for these eight (os, arch) pairs.
        // i686 on macOS has no asset, so it has no pin.
        for (os, arch) in [
            (Os::Mac, Architecture::Aarch64),
            (Os::Mac, Architecture::X8664),
            (Os::Linux, Architecture::Aarch64),
            (Os::Linux, Architecture::X86),
            (Os::Linux, Architecture::X8664),
            (Os::Windows, Architecture::Aarch64),
            (Os::Windows, Architecture::X86),
            (Os::Windows, Architecture::X8664),
        ] {
            let h = uv_sha256(os, arch).unwrap_or_else(|| panic!("{os:?} {arch:?}"));
            assert_eq!(h.len(), 64, "{os:?} {arch:?}");
            assert!(h.chars().all(|c| c.is_ascii_hexdigit()), "{os:?} {arch:?}");
        }
        // A known pin catches an edit to the table, not a format error.
        assert_eq!(
            uv_sha256(Os::Linux, Architecture::X8664),
            Some("9167d72b3319674b6303c4cbe071854bba13ebdf3d76b1a7cbdc175471fb66d6")
        );
        assert!(uv_sha256(Os::Mac, Architecture::X86).is_none());
    }

    #[test]
    fn uv_path_matches_the_archive_layout() {
        let work = Path::new("/work");
        // The tar.gz nests uv-TRIPLE/uv.
        let (stem, _) = uv_asset(Os::Linux, Architecture::X8664);
        assert_eq!(
            uv_path(Os::Linux, &stem, work),
            PathBuf::from("/work/uv/uv-x86_64-unknown-linux-gnu/uv")
        );
        let (stem, _) = uv_asset(Os::Mac, Architecture::Aarch64);
        assert_eq!(
            uv_path(Os::Mac, &stem, work),
            PathBuf::from("/work/uv/uv-aarch64-apple-darwin/uv")
        );
        // The Windows zip is flat: uv.exe at the top level.
        let (stem, _) = uv_asset(Os::Windows, Architecture::X8664);
        assert_eq!(
            uv_path(Os::Windows, &stem, work),
            PathBuf::from("/work/uv/uv.exe")
        );
    }

    #[test]
    fn work_env_keeps_every_dir_in_the_work_dir() {
        let work = Path::new("/work");
        let vars = work_env(work);
        // The variables simdref reads to find its data dir on each OS.
        for key in ["HOME", "XDG_DATA_HOME", "LOCALAPPDATA"] {
            assert!(vars.iter().any(|(k, _)| k == key), "missing {key}");
        }
        for (k, v) in vars.iter().filter(|(_, v)| v != "0") {
            assert!(Path::new(v).starts_with(work), "{k}={v} is outside {}", work.display());
        }
        for key in ["UV_PYTHON_INSTALL_REGISTRY", "UV_PYTHON_INSTALL_BIN"] {
            assert!(vars.iter().any(|(k, v)| k == key && v == "0"), "{key} is not 0");
        }
    }

    #[test]
    fn compare_accepts_a_matching_hash() {
        let got = hex(&Sha256::digest(b"uv archive"));
        assert!(compare(&got, &got).is_ok());
    }

    #[test]
    fn compare_rejects_a_bad_hash() {
        let got = hex(&Sha256::digest(b"uv archive"));
        let other = hex(&Sha256::digest(b"other"));
        let err = compare(&other, &got).unwrap_err();
        assert_eq!(err, "checksum mismatch");
        assert!(compare(&got, &got[..32]).is_err());
    }

    fn today_marker_name() -> String {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        format!("update-{}", secs / 86_400)
    }

    /// Work dir with bin/isa and a uv binary. `outcome` returns Some(()) on success, None on failure.
    fn upgrade_rig<R>(outcome: R) -> (
        PathBuf,
        PathBuf,
        PathBuf,
        std::rc::Rc<std::cell::RefCell<Vec<String>>>,
        impl Fn(&Path, &[&str], &[(&str, &str)]) -> Option<()>,
    )
    where
        R: Fn(&Path) -> Option<()>,
    {
        let work = env::temp_dir().join(format!(
            "simdref-rig-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&work);
        fs::create_dir_all(work.join("bin")).unwrap();
        fs::create_dir_all(work.join("uv")).unwrap();
        let isa = work.join("bin/isa");
        let uv = work.join("uv/uv");
        fs::write(&isa, b"#!/bin/sh\n").unwrap();
        fs::write(&uv, b"#!/bin/sh\n").unwrap();
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let log2 = log.clone();
        let run = move |p: &Path, a: &[&str], _env: &[(&str, &str)]| -> Option<()> {
            log2.borrow_mut().push(format!("{} {}", p.display(), a.join(" ")));
            outcome(p)
        };
        (work, isa, uv, log, run)
    }

    #[test]
    fn find_on_path_is_missing_when_no_path_dir_has_the_file() {
        // A missing binary must fail the check before the spawn.
        let dir = env::temp_dir().join(format!("simdref-find-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("tar"), b"#!/bin/sh\n").unwrap();
        assert!(find_on_path("/no-such-dir", "tar").is_none());
        assert_eq!(
            find_on_path(&dir.to_string_lossy(), "tar"),
            Some(dir.join("tar"))
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn upgrade_with_no_marker_runs_upgrade_and_vaddps_and_creates_today_marker() {
        let (work, isa, uv, log, run) = upgrade_rig(|_| Some(()));
        // Yesterday's marker is present but must be cleaned up.
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let yesterday = work.join(format!("update-{}", secs / 86_400 - 1));
        fs::write(&yesterday, b"").unwrap();
        let result = upgrade_with(&work, &isa, &uv, run);
        assert!(result.is_some());
        let cmds = log.borrow();
        assert_eq!(cmds.len(), 2);
        assert!(cmds[0].ends_with("tool upgrade simdref"), "cmd: {}", cmds[0]);
        assert!(cmds[1].ends_with("vaddps --short"), "cmd: {}", cmds[1]);
        drop(cmds);
        assert!(work.join(today_marker_name()).exists());
        assert!(!yesterday.exists());
        fs::remove_dir_all(&work).ok();
    }

    #[test]
    fn upgrade_with_today_marker_runs_nothing() {
        let (work, isa, uv, log, run) = upgrade_rig(|_| Some(()));
        fs::write(work.join(today_marker_name()), b"").unwrap();
        let result = upgrade_with(&work, &isa, &uv, run);
        assert!(result.is_none());
        assert!(log.borrow().is_empty());
        fs::remove_dir_all(&work).ok();
    }

    #[test]
    fn upgrade_with_failed_upgrade_still_runs_vaddps() {
        let (work, isa, uv, log, run) = upgrade_rig(|_| None);
        let result = upgrade_with(&work, &isa, &uv, run);
        assert!(result.is_none());
        // Even a failed upgrade ends with `vaddps --short` on the installed copy.
        assert_eq!(log.borrow().len(), 2);
        assert!(work.join(today_marker_name()).exists());
        fs::remove_dir_all(&work).ok();
    }
}
