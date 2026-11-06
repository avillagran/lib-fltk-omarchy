//! No local IPC on this OS: the app always runs as the primary instance
//! (`crate::instance` reports IPC as unavailable).

use std::io;
use std::path::Path;
use std::time::Duration;

fn unsupported() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, "no local IPC on this OS")
}

pub(crate) struct Endpoint;
pub(crate) struct Listener;
pub(crate) struct Conn;

pub(crate) fn endpoint(_app_id: &str, _runtime: &Path, _create: bool) -> io::Result<Endpoint> {
    Err(unsupported())
}

impl Endpoint {
    pub(crate) fn describe(&self) -> String {
        String::new()
    }
}

pub(crate) fn claim(_ep: &Endpoint) -> io::Result<Option<Listener>> {
    Err(unsupported())
}

pub(crate) fn connect(_ep: &Endpoint, _timeout: Duration) -> io::Result<Conn> {
    Err(unsupported())
}

impl Listener {
    pub(crate) fn accept(&self) -> io::Result<Conn> {
        Err(unsupported())
    }
    pub(crate) fn wake(&self) {}
}

impl Conn {
    pub(crate) fn set_timeout(&mut self, _d: Option<Duration>) -> io::Result<()> {
        Ok(())
    }
    pub(crate) fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(unsupported())
    }
    pub(crate) fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(unsupported())
    }
    pub(crate) fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn raise_native<W: fltk::prelude::WindowExt>(_win: &W) {}
