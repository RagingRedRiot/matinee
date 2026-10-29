//! A launched program, wrapping the standard child so the session can treat
//! launches the same on every platform.

use std::{io, process};

pub struct Child(process::Child);

impl Child {
    pub(super) fn new(child: process::Child) -> Child {
        Child(child)
    }

    /// The process id.
    pub fn id(&self) -> u32 {
        self.0.id()
    }

    /// `Some(description)` once the process has exited, for a diagnostic.
    pub fn exited(&mut self) -> io::Result<Option<String>> {
        Ok(self.0.try_wait()?.map(|status| status.to_string()))
    }

    /// Stop the process and reap it.
    pub fn kill(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
