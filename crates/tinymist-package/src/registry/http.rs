//! Http registry for tinymist.

use std::io::{BufReader, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;
use reqwest::Certificate;
use reqwest::blocking::Response;
use tinymist_std::ImmutPath;
use typst::diag::{PackageResult, StrResult, eco_format};
use typst::syntax::package::{PackageVersion, VersionlessPackageSpec};

use crate::registry::{PREVIEW_NS, PackageIndexEntry, PackageSpecExt};

use super::{
    DEFAULT_REGISTRY, DummyNotifier, Notifier, PackageError, PackageRegistry, PackageSpec,
};

/// The http package registry for typst.ts.
pub struct HttpRegistry {
    /// The path at which local packages (`@local` packages) are stored.
    package_path: Option<ImmutPath>,
    /// The path at which non-local packages (`@preview` packages) should be
    /// stored when downloaded.
    package_cache_path: Option<ImmutPath>,
    /// lazily initialized package storage.
    storage: OnceLock<PackageStorage>,
    /// The path to the certificate file to use for HTTPS requests.
    cert_path: Option<ImmutPath>,
    /// The notifier to use for progress updates.
    notifier: Arc<Mutex<dyn Notifier + Send>>,
    // package_dir_cache: RwLock<HashMap<PackageSpec, Result<ImmutPath, PackageError>>>,
}

impl Default for HttpRegistry {
    fn default() -> Self {
        Self {
            notifier: Arc::new(Mutex::<DummyNotifier>::default()),
            cert_path: None,
            package_path: None,
            package_cache_path: None,

            storage: OnceLock::new(),
            // package_dir_cache: RwLock::new(HashMap::new()),
        }
    }
}

impl std::ops::Deref for HttpRegistry {
    type Target = PackageStorage;

    fn deref(&self) -> &Self::Target {
        self.storage()
    }
}

impl HttpRegistry {
    /// Create a new registry.
    pub fn new(
        cert_path: Option<ImmutPath>,
        package_path: Option<ImmutPath>,
        package_cache_path: Option<ImmutPath>,
    ) -> Self {
        Self {
            cert_path,
            package_path,
            package_cache_path,
            ..Default::default()
        }
    }

    /// Get `typst-kit` implementing package storage
    pub fn storage(&self) -> &PackageStorage {
        self.storage.get_or_init(|| {
            PackageStorage::new(
                self.package_cache_path
                    .clone()
                    .or_else(|| Some(dirs::cache_dir()?.join(DEFAULT_PACKAGES_SUBDIR).into())),
                self.package_path
                    .clone()
                    .or_else(|| Some(dirs::data_dir()?.join(DEFAULT_PACKAGES_SUBDIR).into())),
                self.cert_path.clone(),
                self.notifier.clone(),
            )
        })
    }

    /// Get local path option
    pub fn local_path(&self) -> Option<ImmutPath> {
        self.storage().package_path().cloned()
    }

    /// Get data & cache dir
    pub fn paths(&self) -> Vec<ImmutPath> {
        let data_dir = self.storage().package_path().cloned();
        let cache_dir = self.storage().package_cache_path().cloned();
        data_dir.into_iter().chain(cache_dir).collect::<Vec<_>>()
    }

    /// Set list of packages for testing.
    pub fn test_package_list(&self, f: impl FnOnce() -> Vec<PackageIndexEntry>) {
        self.storage().index.get_or_init(f);
    }
}

impl PackageRegistry for HttpRegistry {
    fn resolve(&self, spec: &PackageSpec) -> Result<ImmutPath, PackageError> {
        self.storage().prepare_package(spec)
    }

    fn packages(&self) -> &[PackageIndexEntry] {
        self.storage().download_index()
    }
}

/// The default packages sub directory within the package and package cache
/// paths.
pub const DEFAULT_PACKAGES_SUBDIR: &str = "typst/packages";

/// Holds information about where packages should be stored and downloads them
/// on demand, if possible.
pub struct PackageStorage {
    /// The path at which non-local packages should be stored when downloaded.
    package_cache_path: Option<ImmutPath>,
    /// The path at which local packages are stored.
    package_path: Option<ImmutPath>,
    /// The downloader used for fetching the index and packages.
    cert_path: Option<ImmutPath>,
    /// The cached index of the preview namespace.
    index: OnceLock<Vec<PackageIndexEntry>>,
    /// Whether one caller has reserved the background index download.
    index_prefetch_claimed: AtomicBool,
    notifier: Arc<Mutex<dyn Notifier + Send>>,
}

impl PackageStorage {
    /// Creates a new package storage for the given package paths.
    /// It doesn't fallback directories, thus you can disable the related
    /// storage by passing `None`.
    pub fn new(
        package_cache_path: Option<ImmutPath>,
        package_path: Option<ImmutPath>,
        cert_path: Option<ImmutPath>,
        notifier: Arc<Mutex<dyn Notifier + Send>>,
    ) -> Self {
        Self {
            package_cache_path,
            package_path,
            cert_path,
            notifier,
            index: OnceLock::new(),
            index_prefetch_claimed: AtomicBool::new(false),
        }
    }

    /// Returns the path at which non-local packages should be stored when
    /// downloaded.
    pub fn package_cache_path(&self) -> Option<&ImmutPath> {
        self.package_cache_path.as_ref()
    }

    /// Returns the path at which local packages are stored.
    pub fn package_path(&self) -> Option<&ImmutPath> {
        self.package_path.as_ref()
    }

    /// Make a package available in the on-disk cache.
    pub fn prepare_package(&self, spec: &PackageSpec) -> PackageResult<ImmutPath> {
        let subdir = format!("{}/{}/{}", spec.namespace, spec.name, spec.version);

        if let Some(packages_dir) = &self.package_path {
            let dir = packages_dir.join(&subdir);
            if dir.exists() {
                return Ok(dir.into());
            }
        }

        if let Some(cache_dir) = &self.package_cache_path {
            let dir = cache_dir.join(&subdir);
            if dir.exists() {
                return Ok(dir.into());
            }

            // Download from network if it doesn't exist yet.
            if spec.is_preview() {
                self.download_package(spec, &dir)?;
                if dir.exists() {
                    return Ok(dir.into());
                }
            }
        }

        Err(PackageError::NotFound(spec.clone()))
    }

    /// Try to determine the latest version of a package.
    pub fn determine_latest_version(
        &self,
        spec: &VersionlessPackageSpec,
    ) -> StrResult<PackageVersion> {
        if spec.is_preview() {
            // For `@preview`, download the package index and find the latest
            // version.
            self.download_index()
                .iter()
                .filter(|entry| entry.package.name == spec.name)
                .map(|entry| entry.package.version)
                .max()
                .ok_or_else(|| eco_format!("failed to find package {spec}"))
        } else {
            // For other namespaces, search locally. We only search in the data
            // directory and not the cache directory, because the latter is not
            // intended for storage of local packages.
            let subdir = format!("{}/{}", spec.namespace, spec.name);
            self.package_path
                .iter()
                .flat_map(|dir| std::fs::read_dir(dir.join(&subdir)).ok())
                .flatten()
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter_map(|path| path.file_name()?.to_string_lossy().parse().ok())
                .max()
                .ok_or_else(|| eco_format!("please specify the desired version"))
        }
    }

    /// Get the cached package index without network access.
    pub fn cached_index(&self) -> Option<&[PackageIndexEntry]> {
        self.index.get().map(Vec::as_slice)
    }

    /// Reserves the one background index prefetch for this storage.
    ///
    /// Returns false if the index is cached or another caller already reserved
    /// it. A successful caller must schedule [`Self::download_index`]. Claim
    /// before scheduling so concurrent requests do not each occupy a blocking
    /// worker waiting for the same index. The claim is permanent, consistent
    /// with `download_index` caching both success and an empty error fallback.
    /// Direct calls to `download_index` remain available independently.
    pub fn try_claim_index_prefetch(&self) -> bool {
        self.cached_index().is_none()
            && self
                .index_prefetch_claimed
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
    }

    /// Download the package index. The result of this is cached for efficiency.
    pub fn download_index(&self) -> &[PackageIndexEntry] {
        self.index.get_or_init(|| {
            let url = format!("{DEFAULT_REGISTRY}/preview/index.json");

            threaded_http(&url, self.cert_path.as_deref(), |resp| {
                let reader = match resp.and_then(|r| r.error_for_status()) {
                    Ok(response) => response,
                    Err(err) => {
                        // todo: silent error
                        log::error!("Failed to fetch package index: {err} from {url}");
                        return vec![];
                    }
                };

                match read_package_index(reader) {
                    Ok(entry) => entry,
                    Err(err) => {
                        log::error!("Failed to parse package index: {err} from {url}");
                        vec![]
                    }
                }
            })
            .unwrap_or_default()
        })
    }

    /// Download a package over the network.
    ///
    /// # Panics
    /// Panics if the package spec namespace isn't `preview`.
    pub fn download_package(&self, spec: &PackageSpec, package_dir: &Path) -> PackageResult<()> {
        assert!(spec.is_preview(), "only preview packages can be downloaded");

        let url = format!(
            "{DEFAULT_REGISTRY}/preview/{}-{}.tar.gz",
            spec.name, spec.version
        );

        self.notifier.lock().downloading(spec);
        threaded_http(&url, self.cert_path.as_deref(), |resp| {
            let reader = match resp.and_then(|r| r.error_for_status()) {
                Ok(response) => response,
                Err(err) if matches!(err.status().map(|s| s.as_u16()), Some(404)) => {
                    return Err(PackageError::NotFound(spec.clone()));
                }
                Err(err) => return Err(PackageError::NetworkFailed(Some(eco_format!("{err}")))),
            };

            let decompressed = flate2::read::GzDecoder::new(reader);
            tar::Archive::new(decompressed)
                .unpack(package_dir)
                .map_err(|err| {
                    std::fs::remove_dir_all(package_dir).ok();
                    PackageError::MalformedArchive(Some(eco_format!("{err}")))
                })
        })
        .ok_or_else(|| PackageError::Other(Some(eco_format!("cannot spawn http thread"))))?
    }
}

fn read_package_index(reader: impl Read) -> serde_json::Result<Vec<PackageIndexEntry>> {
    // serde_json reads individual bytes from an unbuffered reader. On a
    // blocking HTTP response, each read crosses reqwest's async bridge.
    let mut entries: Vec<PackageIndexEntry> = serde_json::from_reader(BufReader::new(reader))?;
    for entry in &mut entries {
        entry.namespace = PREVIEW_NS.into();
    }
    Ok(entries)
}

pub(crate) fn threaded_http<T: Send + Sync>(
    url: &str,
    cert_path: Option<&Path>,
    f: impl FnOnce(Result<Response, reqwest::Error>) -> T + Send + Sync,
) -> Option<T> {
    std::thread::scope(|s| {
        s.spawn(move || {
            let client_builder = reqwest::blocking::Client::builder();

            let client = if let Some(cert_path) = cert_path {
                let cert = std::fs::read(cert_path)
                    .ok()
                    .and_then(|buf| Certificate::from_pem(&buf).ok());
                if let Some(cert) = cert {
                    client_builder.add_root_certificate(cert).build().unwrap()
                } else {
                    client_builder.build().unwrap()
                }
            } else {
                client_builder.build().unwrap()
            };

            f(client.get(url).send())
        })
        .join()
        .ok()
    })
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;

    fn storage() -> PackageStorage {
        PackageStorage::new(None, None, None, Arc::new(Mutex::new(DummyNotifier)))
    }

    #[test]
    fn concurrent_index_prefetch_has_one_owner() {
        let storage = storage();
        let owners = std::thread::scope(|scope| {
            let threads = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        (0..32)
                            .filter(|_| storage.try_claim_index_prefetch())
                            .count()
                    })
                })
                .collect::<Vec<_>>();
            threads
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .sum::<usize>()
        });
        assert_eq!(owners, 1);
        assert!(storage.cached_index().is_none());
        assert!(!storage.try_claim_index_prefetch());
    }

    #[test]
    fn cached_empty_index_does_not_schedule_prefetch() {
        let storage = storage();
        storage.index.set(Vec::new()).unwrap();
        assert!(!storage.try_claim_index_prefetch());
        assert!(storage.download_index().is_empty());
    }

    #[test]
    fn package_index_reads_are_buffered_and_preserve_entries() {
        struct CountReads<'a> {
            body: &'a [u8],
            calls: usize,
        }
        impl Read for CountReads<'_> {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                self.calls += 1;
                self.body.read(buffer)
            }
        }

        let description = "A package with a long description. ".repeat(128);
        let body = serde_json::to_vec(
            &(0..16)
                .map(|index| {
                    serde_json::json!({
                        "name": format!("buffered-{index}"),
                        "version": "0.1.0",
                        "entrypoint": "lib.typ",
                        "description": description,
                        "updatedAt": 0,
                    })
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let mut reader = CountReads {
            body: &body,
            calls: 0,
        };
        let entries = read_package_index(&mut reader).unwrap();

        assert_eq!(entries.len(), 16);
        for (index, entry) in entries.iter().enumerate() {
            assert_eq!(entry.package.name, format!("buffered-{index}"));
            assert_eq!(
                entry.package.description.as_deref(),
                Some(description.as_str())
            );
            assert_eq!(entry.namespace, PREVIEW_NS);
            assert_eq!(entry.package.version, "0.1.0".parse().unwrap());
        }
        assert!(reader.body.is_empty());
        // The old unbuffered parser performs one upstream read per byte.
        // Leave the precise buffer size unspecified while rejecting that cost.
        assert!(reader.calls < 64, "upstream reads: {}", reader.calls);
    }

    #[test]
    fn package_index_preserves_parse_and_read_failures() {
        assert!(
            read_package_index(&b"[{\"name\":"[..])
                .unwrap_err()
                .is_eof()
        );

        struct FailedRead;
        impl Read for FailedRead {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::new(
                    io::ErrorKind::ConnectionReset,
                    "disconnected",
                ))
            }
        }
        let error = read_package_index(FailedRead).unwrap_err();
        assert!(error.is_io());
        assert_eq!(error.io_error_kind(), Some(io::ErrorKind::ConnectionReset));
    }
}
