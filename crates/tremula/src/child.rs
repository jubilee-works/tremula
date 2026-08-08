//! Keeping a spawned process from outliving the code that started it.
//!
//! [`std::process::Child`] deliberately does nothing when it is dropped: the
//! process it names keeps running. For a language pack that is the wrong
//! default. The pack applies mutants to the project's own sources, so one that
//! outlives the run driving it goes on editing the very files the run is about
//! to report on — and a report that says the sources are intact would be
//! describing a tree that is still changing.
//!
//! Every way out of driving a pack therefore goes through this guard, including
//! the ways that are failures.

use std::{
    io,
    process::{Child, ChildStdout, ExitStatus},
};

/// A spawned process that is stopped and collected when this value is dropped,
/// unless it was waited for first.
#[derive(Debug)]
pub struct Reaped {
    /// The process, until it has been collected. `None` afterwards: there is
    /// nothing left to stop, and asking the operating system again would be
    /// asking about a process that no longer exists.
    running: Option<Child>,
}

impl Reaped {
    /// Take responsibility for `child`.
    #[must_use]
    pub fn new(child: Child) -> Self {
        Self {
            running: Some(child),
        }
    }

    /// The process's identifier, while there is a process.
    #[must_use]
    pub fn pid(&self) -> Option<u32> {
        self.running.as_ref().map(Child::id)
    }

    /// The pipe the process writes to, which can be taken once.
    pub fn stdout(&mut self) -> Option<ChildStdout> {
        self.running.as_mut()?.stdout.take()
    }

    /// Wait for the process to finish, which is also what collects it.
    ///
    /// A wait that fails leaves the process this guard's responsibility, so that
    /// dropping the guard still stops it.
    ///
    /// # Errors
    ///
    /// Returns the underlying I/O error when the process cannot be waited for,
    /// or when it has already been collected.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        let Some(child) = self.running.as_mut() else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "the process has already finished and been collected",
            ));
        };
        let status = child.wait()?;
        self.running = None;
        Ok(status)
    }
}

impl Drop for Reaped {
    /// Stop the process and collect it. Both halves matter: killing it without
    /// waiting would leave a process nobody ever collects.
    fn drop(&mut self) {
        if let Some(child) = self.running.as_mut() {
            drop(child.kill());
            drop(child.wait());
        }
    }
}
