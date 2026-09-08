use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;
use tempfile::TempDir;

const POLL_INTERVAL_MILLISECONDS: &str = "200";
const REPORT_TIMEOUT: Duration = Duration::from_secs(30);
// several poll passes, so a half-copied package would be reported if it could be
const HALF_COPIED_PAUSE: Duration = Duration::from_secs(2);
// nothing new may arrive once every package has been reported
const QUIET_PERIOD: Duration = Duration::from_secs(2);
// faster than the poll interval, so no two passes can measure the same size
const APPEND_INTERVAL: Duration = Duration::from_millis(30);
const APPEND_COUNT: usize = 60;
const APPEND_BYTES: usize = 4096;

fn fixture_files() -> Vec<(String, Vec<u8>)> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/valid_smpte");
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(source)
        .unwrap()
        .flatten()
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                std::fs::read(entry.path()).unwrap(),
            )
        })
        .collect();
    // the asset map has to land first, or the folder is not yet a package
    files.sort_by_key(|(name, _)| name != "ASSETMAP.xml");
    files
}

fn write_package(target: &Path, files: &[(String, Vec<u8>)]) {
    std::fs::create_dir_all(target).unwrap();
    for (name, bytes) in files {
        std::fs::write(target.join(name), bytes).unwrap();
    }
}

/// The watcher's stdout, one line at a time, read on a thread so a test can wait
/// with a timeout instead of blocking on a pipe.
struct Watcher {
    child: Child,
    lines: Receiver<String>,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Watcher {
    fn start(directory: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_dcpdoctor"))
            .args([
                "watch",
                directory.to_str().unwrap(),
                "--interval",
                POLL_INTERVAL_MILLISECONDS,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        Self {
            child,
            lines: read_lines(stdout),
        }
    }

    fn next_report(&self) -> String {
        match self.lines.recv_timeout(REPORT_TIMEOUT) {
            Ok(line) => line,
            Err(error) => panic!("no result line from the watcher: {error:?}"),
        }
    }

    fn expect_nothing_reported_yet(&self) {
        if let Ok(line) = self.lines.try_recv() {
            panic!("the watcher reported {line:?} before the package was complete");
        }
    }

    fn expect_no_report(&self, within: Duration) {
        match self.lines.recv_timeout(within) {
            Ok(line) => panic!("the watcher reported {line:?} a second time"),
            Err(RecvTimeoutError::Timeout) => {}
            Err(error) => panic!("the watcher stopped: {error:?}"),
        }
    }
}

fn read_lines(stdout: ChildStdout) -> Receiver<String> {
    let (sender, receiver) = channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if sender.send(line).is_err() {
                return;
            }
        }
    });
    receiver
}

#[test]
fn every_package_that_lands_is_reported_once_with_its_verdict() {
    let ingest = TempDir::new().unwrap();
    let files = fixture_files();
    let watcher = Watcher::start(ingest.path());

    write_package(&ingest.path().join("good"), &files);
    let report = watcher.next_report();
    assert!(
        report.contains("good") && report.contains("PASS"),
        "the valid package was not reported as passing: {report}"
    );

    // one flipped byte in the sound essence, so its PKL hash no longer matches
    let mut broken = files.clone();
    let sound = broken
        .iter_mut()
        .find(|(name, _)| name == "sound.mxf")
        .expect("the fixture has no sound.mxf");
    sound.1[40] ^= 0xff;
    write_package(&ingest.path().join("broken"), &broken);
    let report = watcher.next_report();
    assert!(
        report.contains("broken") && report.contains("FAIL"),
        "the mutated package was not reported as failing: {report}"
    );

    // the asset map and one asset, then a pause spanning several poll passes
    let slow = ingest.path().join("slow");
    let (first, rest) = files.split_at(2);
    write_package(&slow, first);
    watcher.expect_no_report(HALF_COPIED_PAUSE);
    write_package(&slow, rest);
    let report = watcher.next_report();
    assert!(
        report.contains("slow") && report.contains("PASS"),
        "the half-copied package was not reported whole: {report}"
    );

    // every package has settled, so a further pass must add nothing
    watcher.expect_no_report(QUIET_PERIOD);

    drop(watcher);
}

#[test]
fn a_growing_asset_is_not_validated_until_it_stops_changing() {
    let ingest = TempDir::new().unwrap();
    let files = fixture_files();
    let watcher = Watcher::start(ingest.path());

    let package = ingest.path().join("growing");
    write_package(&package, &files);
    let mut picture = std::fs::File::create(package.join("picture.mxf")).unwrap();
    for _ in 0..APPEND_COUNT {
        picture.write_all(&[0u8; APPEND_BYTES]).unwrap();
        picture.flush().unwrap();
        std::thread::sleep(APPEND_INTERVAL);
    }
    drop(picture);

    watcher.expect_nothing_reported_yet();

    let report = watcher.next_report();
    assert!(
        report.contains("growing"),
        "the package was not reported once the asset stopped growing: {report}"
    );
    watcher.expect_no_report(QUIET_PERIOD);
}
