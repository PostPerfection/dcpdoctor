//! Poll an ingest folder and validate each package once it stops changing.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use crate::assetmap::{AssetMap, ParseXmlFile};

pub const DEFAULT_POLL_INTERVAL_MILLISECONDS: u32 = 5000;

const ASSETMAP_NAMES: [&str; 2] = ["ASSETMAP.xml", "ASSETMAP"];

/// What a package looks like on disk. Two equal readings one interval apart
/// mean whoever was writing it has stopped.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Measurement {
    file_count: usize,
    total_bytes: u64,
    newest_modified: Duration,
}

#[derive(Debug, Clone, Copy)]
struct PackageState {
    measurement: Measurement,
    validated: bool,
}

#[derive(Default)]
struct PackageStates {
    states: HashMap<PathBuf, PackageState>,
}

impl PackageStates {
    fn observe(&mut self, package: &Path, measurement: Measurement) -> bool {
        let Some(state) = self.states.get_mut(package) else {
            self.states.insert(
                package.to_path_buf(),
                PackageState {
                    measurement,
                    validated: false,
                },
            );
            return false;
        };
        if state.validated {
            return false;
        }
        let settled = state.measurement == measurement;
        state.measurement = measurement;
        settled
    }

    fn mark_validated(&mut self, package: &Path) {
        if let Some(state) = self.states.get_mut(package) {
            state.validated = true;
        }
    }

    fn retain_present(&mut self, present: &[PathBuf]) {
        self.states.retain(|package, _| present.contains(package));
    }
}

fn measure(package: &Path) -> Option<Measurement> {
    let mut measurement = Measurement::default();
    let mut directories = vec![package.to_path_buf()];

    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory).ok()?.flatten() {
            let Ok(metadata) = entry.metadata() else {
                return None;
            };
            if metadata.is_dir() {
                directories.push(entry.path());
                continue;
            }
            measurement.file_count += 1;
            measurement.total_bytes += metadata.len();
            let modified = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .unwrap_or_default();
            measurement.newest_modified = measurement.newest_modified.max(modified);
        }
    }

    Some(measurement)
}

fn find_packages(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut packages: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && ASSETMAP_NAMES.iter().any(|name| path.join(name).exists()))
        .collect();
    packages.sort();
    packages
}

fn find_assetmap(package: &Path) -> Option<PathBuf> {
    ASSETMAP_NAMES
        .iter()
        .map(|name| package.join(name))
        .find(|path| path.is_file())
}

/// The asset map names every file the package must hold, so a copy that has not
/// delivered them all is not ready however long it pauses between files.
fn assets_all_present(package: &Path) -> bool {
    let Some(assetmap_path) = find_assetmap(package) else {
        return false;
    };
    let Some(assetmap) = AssetMap::parse(&assetmap_path) else {
        return false;
    };
    assetmap
        .assets
        .iter()
        .all(|asset| contained_path(package, &asset.path).is_some_and(|path| path.is_file()))
}

// an asset map path that is absolute or climbs out names nothing this package owns
fn contained_path(package: &Path, relative: &str) -> Option<PathBuf> {
    let relative = Path::new(relative);
    relative
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
        .then(|| package.join(relative))
}

#[derive(Default)]
pub struct DirectoryWatch {
    states: PackageStates,
}

impl DirectoryWatch {
    pub fn new() -> Self {
        Self::default()
    }

    /// One poll pass. Validates every package that holds all its asset map
    /// names, measured the same as it did on the previous pass, and has not been
    /// validated before, so a package still being copied in is left alone and no
    /// package is reported twice.
    pub fn poll_once(
        &mut self,
        directory: &Path,
        opts: &crate::VerifyOptions,
        on_result: &impl Fn(&Path, &crate::VerifyResult),
    ) {
        let packages = find_packages(directory);
        self.states.retain_present(&packages);

        for package in &packages {
            let Some(measurement) = measure(package) else {
                continue;
            };
            let settled = self.states.observe(package, measurement);
            if !settled || !assets_all_present(package) {
                continue;
            }
            self.states.mark_validated(package);
            tracing::info!("validating {}", package.display());
            let result = crate::verify(package, opts);
            on_result(package, &result);
        }
    }
}

/// Watch a directory for new DCPs and validate each one as it lands.
///
/// Blocks forever, polling every `poll_interval_ms`.
pub fn watch_directory(
    directory: &Path,
    opts: &crate::VerifyOptions,
    on_result: impl Fn(&Path, &crate::VerifyResult),
    poll_interval_ms: u32,
) {
    let interval = Duration::from_millis(u64::from(poll_interval_ms));
    tracing::info!(
        "watching {} for new DCPs, polling every {poll_interval_ms}ms",
        directory.display()
    );

    let mut watch = DirectoryWatch::new();
    loop {
        watch.poll_once(directory, opts, &on_result);
        std::thread::sleep(interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measurement(total_bytes: u64) -> Measurement {
        Measurement {
            file_count: 3,
            total_bytes,
            newest_modified: Duration::from_secs(total_bytes),
        }
    }

    #[test]
    fn a_package_is_validated_only_after_two_equal_measurements() {
        let mut states = PackageStates::default();
        let package = Path::new("/ingest/feature");

        assert!(!states.observe(package, measurement(10)));
        assert!(!states.observe(package, measurement(20)));
        assert!(!states.observe(package, measurement(30)));
        assert!(states.observe(package, measurement(30)));
    }

    #[test]
    fn a_package_already_validated_is_never_validated_again() {
        let mut states = PackageStates::default();
        let package = Path::new("/ingest/feature");

        assert!(!states.observe(package, measurement(10)));
        assert!(states.observe(package, measurement(10)));
        states.mark_validated(package);

        assert!(!states.observe(package, measurement(10)));
        assert!(!states.observe(package, measurement(10)));
    }

    #[test]
    fn a_package_that_gains_a_file_is_not_settled() {
        let mut states = PackageStates::default();
        let package = Path::new("/ingest/feature");
        let two_files = Measurement {
            file_count: 2,
            total_bytes: 1000,
            newest_modified: Duration::ZERO,
        };
        let three_files = Measurement {
            file_count: 3,
            total_bytes: 1000,
            newest_modified: Duration::ZERO,
        };

        assert!(!states.observe(package, two_files));
        assert!(!states.observe(package, three_files));
        assert!(states.observe(package, three_files));
    }

    #[test]
    fn a_package_that_leaves_the_folder_is_forgotten() {
        let mut states = PackageStates::default();
        let package = Path::new("/ingest/feature");

        assert!(!states.observe(package, measurement(10)));
        states.retain_present(&[]);
        assert!(!states.observe(package, measurement(10)));
    }

    #[test]
    fn only_a_directory_holding_an_assetmap_is_a_package() {
        let ingest = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(ingest.path().join("smpte")).unwrap();
        std::fs::write(ingest.path().join("smpte/ASSETMAP.xml"), "<AssetMap/>").unwrap();
        std::fs::create_dir(ingest.path().join("interop")).unwrap();
        std::fs::write(ingest.path().join("interop/ASSETMAP"), "<AssetMap/>").unwrap();
        std::fs::create_dir(ingest.path().join("scratch")).unwrap();
        std::fs::write(ingest.path().join("ASSETMAP.xml"), "<AssetMap/>").unwrap();

        let packages = find_packages(ingest.path());

        assert_eq!(
            packages,
            vec![ingest.path().join("interop"), ingest.path().join("smpte"),],
            "a folder without an asset map is not a package"
        );
    }

    const ONE_ASSET_ASSETMAP: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:01cfce20-0df4-4393-87a6-bd8b0251e5bd</Id>
  <AssetList>
    <Asset>
      <Id>urn:uuid:11111111-2222-3333-4444-555555555555</Id>
      <ChunkList><Chunk><Path>ASSET_PATH</Path></Chunk></ChunkList>
    </Asset>
  </AssetList>
</AssetMap>"#;

    fn package_naming(asset_path: &str) -> tempfile::TempDir {
        let package = tempfile::TempDir::new().unwrap();
        std::fs::write(
            package.path().join("ASSETMAP.xml"),
            ONE_ASSET_ASSETMAP.replace("ASSET_PATH", asset_path),
        )
        .unwrap();
        package
    }

    #[test]
    fn a_package_missing_an_asset_map_file_is_not_ready() {
        let package = package_naming("pkl.xml");
        assert!(!assets_all_present(package.path()));

        std::fs::write(package.path().join("pkl.xml"), "<PackingList/>").unwrap();
        assert!(assets_all_present(package.path()));
    }

    #[test]
    fn an_asset_path_that_climbs_out_of_the_package_is_never_present() {
        let package = package_naming("../outside.xml");
        std::fs::write(package.path().join("../outside.xml"), "x").unwrap();

        assert!(
            !assets_all_present(package.path()),
            "a file outside the package must not count as delivered"
        );
        assert_eq!(contained_path(Path::new("/ingest/dcp"), "../escape"), None);
        assert_eq!(
            contained_path(Path::new("/ingest/dcp"), "/etc/passwd"),
            None
        );
        assert_eq!(
            contained_path(Path::new("/ingest/dcp"), "pkl.xml"),
            Some(PathBuf::from("/ingest/dcp/pkl.xml"))
        );
    }

    #[test]
    fn a_file_that_grows_changes_the_measurement() {
        let package = tempfile::TempDir::new().unwrap();
        std::fs::write(package.path().join("ASSETMAP.xml"), "<AssetMap/>").unwrap();
        let half = measure(package.path()).unwrap();

        std::fs::write(package.path().join("picture.mxf"), [0u8; 512]).unwrap();
        let whole = measure(package.path()).unwrap();

        assert_ne!(half, whole);
        assert_eq!(whole.file_count, 2);
        assert_eq!(whole.total_bytes, half.total_bytes + 512);
    }
}
