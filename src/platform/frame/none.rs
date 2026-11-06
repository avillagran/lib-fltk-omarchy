//! Neither Unix nor Windows: no shared memory. `crate::frame`'s PNG/BMP
//! file mode still works.

use super::{Kind, Parts, Role, Wait};
use std::io;
use std::path::Path;
use std::time::Duration;

pub(crate) struct MapOs;
pub(crate) struct Bell;
pub(crate) struct Waiter;
#[derive(Clone)]
pub(crate) struct Interrupter;
#[derive(Default)]
pub(crate) struct Remote;
#[derive(Default)]
pub(crate) struct Cleanup;

impl Bell {
    pub(crate) fn ring(&self) {}
}

impl Waiter {
    pub(crate) fn wait(&mut self, _timeout: Option<Duration>) -> Wait {
        Wait::Closed
    }
    pub(crate) fn interrupter(&self) -> Interrupter {
        Interrupter
    }
}

impl Interrupter {
    pub(crate) fn interrupt(&self) {}
}

impl Remote {
    pub(crate) fn share(&self, _cmd: &mut std::process::Command) {}
    pub(crate) fn release(&mut self) {}
}

pub(crate) fn unmap(_ptr: *mut u8, _len: usize) {}

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "shared frame surfaces are not supported on this OS",
    )
}

pub(crate) fn create(_kind: Kind, _len: usize, _base: &Path, _role: Role) -> io::Result<Parts> {
    Err(unsupported())
}

pub(crate) fn open(_handle: &str, _role: Role) -> io::Result<Parts> {
    Err(unsupported())
}
