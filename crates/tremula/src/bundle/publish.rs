//! Where a bundle is assembled, and how it comes to stand at the name it was asked for.
//!
//! # Why it is assembled somewhere else
//!
//! A bundle is a thing that gets copied and sent, and a half-written one looks exactly
//! like a whole one. So everything is written into a staging directory beside where the
//! bundle goes — a sibling, so that publishing it stays a rename within one filesystem —
//! and the published path appears complete or not at all.
//!
//! # Why the staging directory is named after the process
//!
//! Because two bundles of one run can be asked for at one path at one moment, and a
//! staging name they shared would be a name each of them took away from the other. Named
//! after the process and the call within it, a staging directory belongs to the attempt
//! assembling it and nothing else looks in it.
//!
//! # Why the destination is created rather than checked
//!
//! Asking whether something is at the output path and then moving onto it are two
//! operations, and the gap between them is exactly long enough for another attempt to
//! publish its own bundle there — which the move would then destroy. Asking also follows a
//! link, so it answers about wherever the link points rather than about the path.
//!
//! Creating the directory is one operation the filesystem either performs or refuses, so
//! the refusal *is* the claim: whoever created it has the name. What makes that enough
//! rather than a claim on an empty directory somebody might read is the move onto it —
//! POSIX renames a directory onto an empty directory, so the published path goes from
//! empty to finished in one step and never holds part of a bundle.

use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::bundle::failures::BundleFailure;

/// What every staging name carries before the part that makes it one attempt's own.
const STAGING_SUFFIX: &str = ".part";

/// The directory one attempt assembles a bundle in.
#[derive(Debug)]
pub struct Staging {
    /// Where everything is written. Absolute, because `git apply --check` is run with the
    /// run's snapshot as its working directory and handed the path of a patch to read.
    path: PathBuf,
    /// The same directory as the caller would spell it, which is what a person is told
    /// about. A message naming `/private/var/...` when the caller typed `bundle` is a
    /// message about somewhere they have never been.
    given: PathBuf,
}

impl Staging {
    /// Make the directory this attempt will assemble a bundle in.
    ///
    /// # Errors
    ///
    /// Returns [`BundleFailure::Unwritable`] when it cannot be created.
    pub fn beside(out: &Path) -> Result<Self, BundleFailure> {
        let given = named_beside(out);
        let path = absolute(&given)?;
        let unwritable = |reason: String| BundleFailure::Unwritable {
            path: given.clone(),
            reason,
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| unwritable(err.to_string()))?;
        }
        match fs::create_dir(&path) {
            Ok(()) => {}
            // The name is this process's own, so whatever is under it was left by a process
            // that died and whose identifier this one was later given. Nothing there can be
            // work anybody is still doing.
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                fs::remove_dir_all(&path).map_err(|err| unwritable(err.to_string()))?;
                fs::create_dir(&path).map_err(|err| unwritable(err.to_string()))?;
            }
            Err(err) => return Err(unwritable(err.to_string())),
        }
        Ok(Self { path, given })
    }

    /// Where this attempt writes.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Take the name `out` and put the finished bundle at it.
    ///
    /// # Errors
    ///
    /// Returns [`BundleFailure::BundleExists`] when anything is already at `out`, and
    /// [`BundleFailure::Unwritable`] when the move could not be made.
    pub fn publish(&self, out: &Path) -> Result<(), BundleFailure> {
        let unwritable = |reason: String| BundleFailure::Unwritable {
            path: out.to_path_buf(),
            reason,
        };
        if let Some(parent) = out.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|err| unwritable(err.to_string()))?;
        }
        match fs::create_dir(out) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                return Err(BundleFailure::BundleExists {
                    path: out.to_path_buf(),
                });
            }
            Err(err) => return Err(unwritable(err.to_string())),
        }
        fs::rename(&self.path, out).map_err(|err| {
            // The claim was this attempt's, so giving it up is this attempt's to do.
            drop(fs::remove_dir(out));
            unwritable(err.to_string())
        })
    }

    /// Take the staging directory away, in whatever state it is in.
    pub fn discard(self) {
        drop(fs::remove_dir_all(&self.path));
    }

    /// One failure of this attempt, with any path inside the staging directory spelled the
    /// way the caller spelled the staging directory itself.
    #[must_use]
    pub fn as_the_caller_said(&self, failure: BundleFailure) -> BundleFailure {
        match failure {
            BundleFailure::Unwritable { path, reason } => BundleFailure::Unwritable {
                path: self.given_form(&path),
                reason,
            },
            BundleFailure::Unreadable { path, reason } => BundleFailure::Unreadable {
                path: self.given_form(&path),
                reason,
            },
            other => other,
        }
    }

    /// `path` as the caller would have spelled it, when it is one of this attempt's own.
    fn given_form(&self, path: &Path) -> PathBuf {
        match path.strip_prefix(&self.path) {
            Ok(inside) if inside.as_os_str().is_empty() => self.given.clone(),
            Ok(inside) => self.given.join(inside),
            Err(_) => path.to_path_buf(),
        }
    }
}

/// A staging name beside `out` that belongs to this attempt alone: the process, and one
/// call within it, since two threads of one process can package two runs at once.
fn named_beside(out: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);

    let ordinal = NEXT.fetch_add(1, Ordering::Relaxed);
    let mut name = OsString::from(out.as_os_str());
    name.push(format!("{STAGING_SUFFIX}.{}.{ordinal}", process::id()));
    PathBuf::from(name)
}

/// A path that survives a change of working directory.
fn absolute(path: &Path) -> Result<PathBuf, BundleFailure> {
    std::path::absolute(path).map_err(|err| BundleFailure::PathUnresolvable {
        path: path.to_path_buf(),
        reason: err.to_string(),
    })
}
