//! Integration tests for the `vivelite-xtask` binary: argument parsing,
//! subcommand dispatch, and error reporting.
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

const ROOTFS_SHA256: &str = "64c87a3f366ec7a3ee30c5298c860e4f301a6e271381647c299a7c3eb6d38eb3";

struct Fixture {
    root: tempfile::TempDir,
    tools: PathBuf,
    work: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let tools = root.path().join("tools");
        let work = root.path().join("work");
        fs::create_dir_all(&tools).unwrap();
        fs::create_dir_all(&work).unwrap();
        let mut f = Self { root, tools, work };
        f.install_tools();
        f
    }

    fn script(&self, name: &str, body: &str) {
        let path = self.tools.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn install_tools(&mut self) {
        self.script(
            "curl",
            r#"
if [ -n "$FAKE_CURL_FAIL" ]; then echo 'fake curl failed' >&2; exit 7; fi
if [ -n "$FAKE_CURL_NOOP" ]; then exit 0; fi
out=""
while [ $# -gt 0 ]; do
    case "$1" in
        --output) out="$2"; shift 2 ;;
        *) shift ;;
    esac
done
if [ "$FAKE_STAGE_SCENARIO" = bundledl ] && [ -n "$out" ]; then
    echo 'fake curl failed' >&2
    exit 7
fi
if [ -n "$out" ]; then
    printf 'fake bundle' > "$out"
else
    case "$FAKE_STAGE_SCENARIO" in
        badutf8) printf '\377' ;;
        empty) printf '{"candidates": []}' ;;
        *) printf '{"candidates":[{"buildid":"b1","version":"0.4.5","update_path":"vr/b1/deckard.raucb","chunks_store_path":"vr/chunks.castr"}]}' ;;
    esac
fi
"#,
        );
        self.script(
            "rauc",
            "if [ -n \"$FAKE_RAUC_FAIL\" ]; then echo 'bad signature' >&2; exit 4; fi; exit 0",
        );
        self.script(
            "unsquashfs",
            r#"
if [ -n "$FAKE_UNSQUASHFS_FAIL" ]; then echo 'squashfs failure' >&2; exit 5; fi
dest=""
while [ $# -gt 0 ]; do
    case "$1" in
        -d) dest="$2"; shift 2 ;;
        -*) shift ;;
        *) break ;;
    esac
done
shift
for p in "$@"; do
    target="$dest/$p"
    mkdir -p "$(dirname "$target")"
    case "$p" in
        manifest.raucm)
            case "$FAKE_STAGE_SCENARIO" in
                incompatible) printf '[update]\ncompatible=steamos-x86\n[image.rootfs]\nfilename=rootfs.img.caibx\n' > "$target" ;;
                no-compatible) printf '[image.rootfs]\nfilename=rootfs.img.caibx\n' > "$target" ;;
                badmanifest) printf '[broken\n' > "$target" ;;
                bad-filename) printf '[update]\ncompatible=steamos-aarch64\n[image.rootfs]\nfilename=../escape.caibx\n' > "$target" ;;
                no-rootfs) printf '[update]\ncompatible=steamos-aarch64\n[image.boot]\nfilename=boot.caibx\n' > "$target" ;;
                no-sha) printf '[update]\ncompatible=steamos-aarch64\n[image.rootfs]\nfilename=rootfs.img.caibx\n' > "$target" ;;
                missing-caibx) printf '[update]\ncompatible=steamos-aarch64\n[image.rootfs]\nfilename=missing.caibx\n' > "$target" ;;
                bad-sha) printf '[update]\ncompatible=steamos-aarch64\n[image.rootfs]\nfilename=rootfs.img.caibx\nsha256=00\n' > "$target" ;;
                *) printf '[update]\ncompatible=steamos-aarch64\n[image.rootfs]\nfilename=rootfs.img.caibx\nsha256=__ROOTFS_SHA256__\nsize=12\n' > "$target" ;;
            esac
            ;;
        rootfs.img.caibx)
            [ "$FAKE_STAGE_SCENARIO" = missing-caibx ] || printf 'fake index' > "$target"
            ;;
        *) printf 'fake component' > "$target" ;;
    esac
done
"#
            .replace("__ROOTFS_SHA256__", ROOTFS_SHA256)
            .as_str(),
        );
        self.script(
            "desync",
            r#"
if [ -n "$FAKE_DESYNC_FAIL" ]; then echo 'desync failed' >&2; exit 6; fi
if [ "$1" = info ]; then
    if [ -n "$FAKE_DESYNC_NOINFO" ]; then echo '{}'; else echo '{"unique": 1, "dedup-size-not-in-seed": 1}'; fi
    exit 0
fi
out=""
for arg in "$@"; do out="$arg"; done
if [ -n "$FAKE_DESYNC_EMPTY" ]; then : > "$out"; else printf 'rootfs-bytes' > "$out"; fi
"#,
        );
        self.script(
            "debugfs",
            r#"
req="$2"
if [ "$FAKE_DEBUGFS_SCENARIO" = skip ]; then
    case "$req" in dump\ *|rdump\ *) exit 0 ;; esac
fi
case "$FAKE_DEBUGFS_SCENARIO" in
    notfound) echo 'File not found by ext2_lookup'; exit 0 ;;
    failure) echo 'debugfs failure' >&2; exit 8 ;;
esac
case "$req" in
    stat\ *)
        inner=${req#stat }
        case "$inner" in
            usr/lib/wine|usr/lib/pressure-vessel|usr/share/fex) echo 'Type: directory' ;;
            *) echo 'Type: regular file' ;;
        esac
        ;;
    rdump\ *)
        inner=$(echo "$req" | awk '{print $2}')
        dest=$(echo "$req" | awk '{print $3}')
        name=${inner##*/}
        mkdir -p "$dest/$name"
        printf 'component' > "$dest/$name/payload"
        ;;
    dump\ *)
        dest=$(echo "$req" | awk '{print $3}')
        printf 'component' > "$dest"
        ;;
esac
"#,
        );
        self.script("openssl", "exit 0");
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_xtask"));
        cmd.args(args)
            .current_dir(self.root.path())
            .env("VIVELITE_WORK", &self.work)
            .env(
                "VIVELITE_RAUC_KEYRING",
                self.root.path().join("trusted-ca.pem"),
            )
            .env("PATH", format!("{}:/usr/bin:/bin", self.tools.display()));
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    fn stage(&self, scenario: &str) -> Output {
        self.command(&["stage", "beta"])
            .env("FAKE_STAGE_SCENARIO", scenario)
            .output()
            .unwrap()
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn help_unknown_and_argument_errors() {
    let f = Fixture::new();
    let empty = f.run(&[]);
    assert!(empty.status.success());
    assert!(stdout(&empty).contains("cargo xtask dev"));
    let help = f.run(&["help"]);
    assert!(help.status.success());
    assert!(stdout(&help).contains("cargo xtask dev"));
    for arg in ["--help", "-h"] {
        assert!(f.run(&[arg]).status.success());
    }
    let unknown = f.run(&["unknown-command"]);
    assert_eq!(unknown.status.code(), Some(2));
    assert!(stderr(&unknown).contains("unknown command"));
    let missing_extract = f.run(&["extract"]);
    assert_eq!(missing_extract.status.code(), Some(2));
    assert!(stderr(&missing_extract).contains("extract needs"));
    let bad_gfr_backend = f.run(&["gfr-probe", "--backend", "adreno"]);
    assert_eq!(bad_gfr_backend.status.code(), Some(2));
    assert!(stderr(&bad_gfr_backend).contains("expected auto, amd, or portable"));
    assert!(f.run(&["verify"]).status.success());
    let no_url = f
        .command(&["fetch"])
        .env_remove("VIVELITE_BUNDLE_URL")
        .output()
        .unwrap();
    assert_eq!(no_url.status.code(), Some(2));
    assert!(stderr(&no_url).contains("fetch needs"));
}

#[test]
fn fetch_verify_extract_and_doctor_paths() {
    let f = Fixture::new();
    let fetched = f.run(&["fetch", "https://example.invalid/bundle.raucb"]);
    assert!(fetched.status.success(), "{}", stderr(&fetched));
    assert!(stdout(&fetched).contains("https://example.invalid/bundle.raucb"));

    let verified = f.run(&["verify"]);
    assert!(verified.status.success(), "{}", stderr(&verified));
    let wrong_sha = "00".repeat(32);
    let wrong_hash = f
        .command(&["verify", "--sha256", &wrong_sha])
        .output()
        .unwrap();
    assert_eq!(wrong_hash.status.code(), Some(2));
    assert!(stderr(&wrong_hash).contains("sha256 mismatch"));
    let extracted = f.run(&["extract", "slot.img"]);
    assert!(extracted.status.success(), "{}", stderr(&extracted));
    assert!(stdout(&extracted).contains("staged 5 components"));

    let mut doctor = f.command(&["doctor"]);
    doctor.env(
        "VIVELITE_BUNDLE_URL",
        "https://example.invalid/bundle.raucb",
    );
    let doctor = doctor.output().unwrap();
    assert!(doctor.status.success(), "{}", stderr(&doctor));

    let failing_fetch = f
        .command(&["fetch", "https://example.invalid/fail"])
        .env("FAKE_CURL_FAIL", "1")
        .output()
        .unwrap();
    assert_eq!(failing_fetch.status.code(), Some(2));
    assert!(stderr(&failing_fetch).contains("download failed"));

    let failing_verify = f
        .command(&["verify"])
        .env("FAKE_RAUC_FAIL", "1")
        .output()
        .unwrap();
    assert_eq!(failing_verify.status.code(), Some(2));
    assert!(stderr(&failing_verify).contains("bad signature"));

    let failing_extract = f
        .command(&["extract", "slot.img"])
        .env("FAKE_UNSQUASHFS_FAIL", "1")
        .output()
        .unwrap();
    assert_eq!(failing_extract.status.code(), Some(2));
    assert!(stderr(&failing_extract).contains("squashfs failure"));
}

#[test]
fn doctor_reports_missing_dependencies_and_default_work_root() {
    let root = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("doctor")
        .current_dir(root.path())
        .env("PATH", "/nonexistent")
        .env_remove("VIVELITE_WORK")
        .env_remove("VIVELITE_BUNDLE_URL")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("check(s) failed"));
    assert!(stderr(&output).contains("work/bundles/steamos.raucb"));
}

#[test]
fn stage_success_and_manifest_failures() {
    let f = Fixture::new();
    let default_branch = f.run(&["stage"]);
    assert!(
        default_branch.status.success(),
        "{}\n{}",
        stdout(&default_branch),
        stderr(&default_branch)
    );
    let ok = f.stage("ok");
    assert!(ok.status.success(), "{}\n{}", stdout(&ok), stderr(&ok));
    assert!(stdout(&ok).contains("staged 5 SteamOS components"));

    let no_sha = f.stage("no-sha");
    assert!(no_sha.status.success(), "{}", stderr(&no_sha));

    for (scenario, expected) in [
        ("incompatible", "expected steamos-aarch64"),
        ("no-compatible", "manifest lacks compatible"),
        ("no-rootfs", "manifest lacks an [image.rootfs] section"),
        ("bad-filename", "unsafe image filename"),
        ("missing-caibx", "bundle does not carry missing.caibx"),
        ("bad-sha", "sha256 mismatch"),
    ] {
        let failed = f.stage(scenario);
        assert_eq!(
            failed.status.code(),
            Some(2),
            "{scenario}: {}",
            stderr(&failed)
        );
        assert!(
            stderr(&failed).contains(expected),
            "{scenario}: {}",
            stderr(&failed)
        );
    }
}

#[test]
fn fetch_reports_missing_output_after_successful_curl() {
    let f = Fixture::new();
    let out = f
        .command(&["fetch", "https://example.invalid/noop.raucb"])
        .env("FAKE_CURL_NOOP", "1")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("No such file or directory"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn fetch_reports_work_dir_creation_failure() {
    let f = Fixture::new();
    let blocker = f.root.path().join("blocker");
    fs::write(&blocker, "not a directory").unwrap();
    let out = f
        .command(&["fetch", "https://example.invalid/b.raucb"])
        .env("VIVELITE_WORK", blocker.join("work"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(stderr(&out).contains("blocker"), "{}", stderr(&out));
}

#[test]
fn stage_reports_blocked_unpack_and_chunk_cache_dirs() {
    let f = Fixture::new();
    let bundles = f.work.join("bundles");
    fs::create_dir_all(&bundles).unwrap();
    fs::write(bundles.join("b1.d"), "blocks the unpack dir").unwrap();
    let out = f.stage("ok");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(stderr(&out).contains("b1.d"), "{}", stderr(&out));

    let _ = fs::remove_dir_all(&bundles);
    let cache = f.work.join("cache");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("desync"), "blocks the chunk cache").unwrap();
    let out = f.stage("ok");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(stderr(&out).contains("desync"), "{}", stderr(&out));
}

#[test]
fn stage_reports_manifest_parse_and_incomplete_layout() {
    let f = Fixture::new();
    let bad = f.stage("badmanifest");
    assert_eq!(bad.status.code(), Some(2), "{}", stderr(&bad));
    assert!(stderr(&bad).contains("bad manifest"), "{}", stderr(&bad));

    let _ = fs::remove_dir_all(f.work.join("bundles"));
    let _ = fs::remove_dir_all(f.work.join("staged"));
    let _ = fs::remove_file(f.work.join("rootfs-b1.img"));
    let incomplete = f
        .command(&["stage", "stable"])
        .env("FAKE_DEBUGFS_SCENARIO", "skip")
        .output()
        .unwrap();
    assert_eq!(incomplete.status.code(), Some(2), "{}", stderr(&incomplete));
    assert!(
        stderr(&incomplete).contains("staged SteamOS layout is incomplete"),
        "{}",
        stderr(&incomplete)
    );
}

#[test]
fn stage_reports_bundle_download_failure() {
    let f = Fixture::new();
    let out = f
        .command(&["stage", "stable"])
        .env("FAKE_STAGE_SCENARIO", "bundledl")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("download failed:"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn curl_spawn_failures_are_reported() {
    let f = Fixture::new();
    let fetch = f
        .command(&["fetch", "https://example.invalid/b.raucb"])
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert_eq!(fetch.status.code(), Some(2));
    assert!(stderr(&fetch).contains("could not run curl"));

    let stage = f
        .command(&["stage", "stable"])
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert_eq!(stage.status.code(), Some(2));
    assert!(stderr(&stage).contains("could not run curl"));
}

#[test]
fn stage_reports_tool_and_feed_failures() {
    let f = Fixture::new();
    let cases: [(&str, &str, &str); 4] = [
        ("FAKE_CURL_FAIL", "1", "could not fetch"),
        ("FAKE_RAUC_FAIL", "1", "bad signature"),
        ("FAKE_UNSQUASHFS_FAIL", "1", "squashfs failure"),
        ("FAKE_DEBUGFS_SCENARIO", "failure", "debugfs failure"),
    ];
    for (var, value, expected) in cases {
        let out = f
            .command(&["stage", "stable"])
            .env(var, value)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{var}: {}", stderr(&out));
        assert!(stderr(&out).contains(expected), "{var}: {}", stderr(&out));
        let _ = fs::remove_dir_all(f.work.join("bundles"));
        let _ = fs::remove_dir_all(f.work.join("staged"));
        let _ = fs::remove_file(f.work.join("rootfs-b1.img"));
    }
}

#[test]
fn stage_reports_feed_parse_problems() {
    let f = Fixture::new();
    for (scenario, expected) in [
        ("badutf8", "non-utf8 response"),
        ("empty", "candidates array is empty"),
    ] {
        let out = f
            .command(&["stage", "stable"])
            .env("FAKE_STAGE_SCENARIO", scenario)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{scenario}");
        assert!(
            stderr(&out).contains(expected),
            "{scenario}: {}",
            stderr(&out)
        );
    }
}

#[test]
fn stage_reports_desync_and_staging_problems() {
    let f = Fixture::new();
    let cases: [(&str, &str, &str); 4] = [
        ("FAKE_DESYNC_NOINFO", "1", "missing \"unique\""),
        ("FAKE_DESYNC_FAIL", "1", "desync failed"),
        ("FAKE_DESYNC_EMPTY", "1", "is empty after reconstruction"),
        ("FAKE_DEBUGFS_SCENARIO", "notfound", "not found"),
    ];
    for (var, value, expected) in cases {
        let out = f
            .command(&["stage", "stable"])
            .env(var, value)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{var}: {}", stderr(&out));
        assert!(stderr(&out).contains(expected), "{var}: {}", stderr(&out));
        let _ = fs::remove_dir_all(f.work.join("bundles"));
        let _ = fs::remove_dir_all(f.work.join("staged"));
        let _ = fs::remove_file(f.work.join("rootfs-b1.img"));
    }
}

#[test]
fn dev_command_uses_configured_cargo_and_ui_fails_fast_without_display() {
    let f = Fixture::new();
    let dev = f
        .command(&["dev"])
        .env("CARGO", "/bin/true")
        .output()
        .unwrap();
    assert!(dev.status.success(), "{}", stderr(&dev));
    assert!(stdout(&dev).contains("dev-check passed"));

    let dev_fail = f
        .command(&["dev"])
        .env("CARGO", "/bin/false")
        .output()
        .unwrap();
    assert_eq!(dev_fail.status.code(), Some(2));
    assert!(stderr(&dev_fail).contains("build workspace"));

    let ui = f
        .command(&["ui"])
        .env("DISPLAY", ":vivelite-test-no-display")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    assert_eq!(ui.status.code(), Some(2));
    assert!(stderr(&ui).contains("could not launch dashboard"));

    let overlay = f
        .command(&["device-ui"])
        .env("DISPLAY", ":vivelite-test-no-display")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    assert_eq!(overlay.status.code(), Some(2));
    assert!(stderr(&overlay).contains("could not launch device overlay"));
}

#[test]
fn fetch_and_extract_argument_and_io_edges() {
    let f = Fixture::new();
    let from_env = f
        .command(&["fetch"])
        .env("VIVELITE_BUNDLE_URL", "https://example.invalid/env.raucb")
        .output()
        .unwrap();
    assert!(from_env.status.success(), "{}", stderr(&from_env));
    assert!(stdout(&from_env).contains("https://example.invalid/env.raucb"));

    let no_url = f
        .command(&["fetch"])
        .env_remove("VIVELITE_BUNDLE_URL")
        .output()
        .unwrap();
    assert_eq!(no_url.status.code(), Some(2));
    assert!(stderr(&no_url).contains("fetch needs <bundle-url>"));

    let blocked = f
        .command(&["extract", "slot.img"])
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert_eq!(blocked.status.code(), Some(2));
    assert!(
        stderr(&blocked).contains("could not run"),
        "{}",
        stderr(&blocked)
    );
}
