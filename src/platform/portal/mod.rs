//! xdg-desktop-portal's `org.freedesktop.portal.FileChooser` for
//! [`crate::dialog`] (Linux and BSDs): `OpenFile` and `SaveFile` over the
//! session bus, then the `Response` signal of the request object.
//!
//! The request handle is predictable (`handle_token`), so the match rule
//! for `Response` is added before the call and no answer is lost. The
//! portal closes a request when its caller leaves the bus, so the whole
//! exchange runs on one connection held by a worker thread
//! ([`crate::dialog`] keeps the UI loop running meanwhile). When the
//! portal service goes away during the dialog (`NameOwnerChanged`), the
//! request fails instead of waiting forever.

pub(crate) mod dbus;

use crate::dialog::{Error, Filter, Pattern, Request};
use dbus::{Connection, Message, Type, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

pub(crate) const DEST: &str = "org.freedesktop.portal.Desktop";
pub(crate) const PATH: &str = "/org/freedesktop/portal/desktop";
pub(crate) const IFACE: &str = "org.freedesktop.portal.FileChooser";
const REQUEST_IFACE: &str = "org.freedesktop.portal.Request";

/// Connecting, authenticating and each method call. The first call may
/// start the portal through D-Bus activation, which takes a few seconds.
const CALL_TIMEOUT: Duration = Duration::from_secs(25);

/// A `handle_token` unique in this process (letters, digits, `_`).
pub(crate) fn token() -> String {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    format!(
        "fltk_omarchy_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// The request object the portal creates for `token`:
/// `/org/freedesktop/portal/desktop/request/SENDER/TOKEN`, where SENDER is
/// our unique name without the `:` and with `.` as `_`.
pub(crate) fn request_path(unique: &str, token: &str) -> String {
    let sender = unique.trim_start_matches(':').replace('.', "_");
    format!("{PATH}/request/{sender}/{token}")
}

fn match_rule(path: &str) -> String {
    format!("type='signal',interface='{REQUEST_IFACE}',member='Response',path='{path}'")
}

fn owner_rule() -> String {
    format!(
        "type='signal',sender='org.freedesktop.DBus',interface='org.freedesktop.DBus',\
         member='NameOwnerChanged',arg0='{DEST}'"
    )
}

/// `a(sa(us))`: each filter's name and its patterns (0 glob, 1 MIME type).
pub(crate) fn filters_value(filters: &[Filter]) -> Value {
    let pattern_t = Type::Struct(vec![Type::Basic(b'u'), Type::Basic(b's')]);
    let filter_t = Type::Struct(vec![
        Type::Basic(b's'),
        Type::Array(Box::new(pattern_t.clone())),
    ]);
    let items = filters
        .iter()
        .filter(|f| !f.patterns().is_empty())
        .map(|f| {
            let patterns = f
                .patterns()
                .iter()
                .map(|p| match p {
                    Pattern::Glob(g) => Value::Struct(vec![Value::U32(0), Value::str(g.clone())]),
                    Pattern::Mime(m) => Value::Struct(vec![Value::U32(1), Value::str(m.clone())]),
                })
                .collect();
            Value::Struct(vec![
                Value::str(f.name()),
                Value::Array(pattern_t.clone(), patterns),
            ])
        })
        .collect();
    Value::Array(filter_t, items)
}

fn path_bytes(p: &std::path::Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    p.as_os_str().as_bytes().to_vec()
}

/// The `OpenFile` or `SaveFile` call for `req`.
pub(crate) fn call_message(req: &Request, parent: &str, token: &str) -> Message {
    let mut opts: Vec<(&str, Value)> = vec![
        ("handle_token", Value::str(token)),
        ("modal", Value::Bool(true)),
    ];
    if let Some(l) = &req.accept_label {
        opts.push(("accept_label", Value::str(l.clone())));
    }
    if !req.save {
        opts.push(("multiple", Value::Bool(req.multiple)));
        opts.push(("directory", Value::Bool(req.directory)));
    }
    let filters = filters_value(&req.filters);
    if matches!(&filters, Value::Array(_, f) if !f.is_empty()) {
        if let Value::Array(_, f) = &filters {
            // The first filter is selected, as in the native choosers.
            opts.push(("current_filter", f[0].clone()));
        }
        opts.push(("filters", filters));
    }
    if req.save {
        if let Some(n) = req.name.as_deref().filter(|n| !n.is_empty()) {
            opts.push(("current_name", Value::str(n)));
        }
    }
    if let Some(dir) = &req.folder {
        opts.push(("current_folder", Value::bytestring(&path_bytes(dir))));
    }
    Message::call(
        DEST,
        PATH,
        IFACE,
        if req.save { "SaveFile" } else { "OpenFile" },
        vec![
            Value::str(parent),
            Value::str(req.title.clone()),
            Value::dict(opts),
        ],
    )
}

/// The paths of a `Response(u response, a{sv} results)` body.
pub(crate) fn parse_response(body: &[Value]) -> Result<Vec<PathBuf>, Error> {
    let code = body.first().and_then(Value::as_u32);
    match code {
        Some(0) => {}
        Some(1) => return Err(Error::Cancelled),
        Some(n) => {
            return Err(Error::Unavailable(format!(
                "the file chooser portal ended the request (response {n})"
            )))
        }
        None => {
            return Err(Error::Unavailable(
                "malformed file chooser portal response".into(),
            ))
        }
    }
    let uris = body.get(1).and_then(|r| r.get("uris"));
    let Some(Value::Array(_, uris)) = uris else {
        return Err(Error::Cancelled); // success without files: nothing picked
    };
    let paths: Vec<PathBuf> = uris
        .iter()
        .filter_map(Value::as_str)
        .filter_map(uri_to_path)
        .collect();
    if paths.is_empty() {
        return Err(Error::Cancelled);
    }
    Ok(paths)
}

/// `file:///a/b%20c` → `/a/b c` (also `file://localhost/…`); None for
/// other schemes and hosts.
pub(crate) fn uri_to_path(uri: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let b = rest.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

fn unavailable(e: dbus::Error) -> Error {
    Error::Unavailable(match &e {
        dbus::Error::Remote(name, _)
            if name.ends_with(".ServiceUnknown")
                || name.ends_with(".NameHasNoOwner")
                || name.ends_with(".UnknownMethod")
                || name.ends_with(".UnknownInterface")
                || name.ends_with(".UnknownObject") =>
        {
            format!("no xdg-desktop-portal file chooser ({e})")
        }
        _ => format!("xdg-desktop-portal: {e}"),
    })
}

/// Runs `req` through the portal on the session bus. Blocks until the user
/// answers: call it from a worker thread.
pub(crate) fn run(req: &Request, parent: &str) -> Result<Vec<PathBuf>, Error> {
    if let Some(addr) = test_bus() {
        return run_at(Some(&addr), req, parent);
    }
    run_at(None, req, parent)
}

/// [`run`] on the bus at `address` (a D-Bus address), or the session bus.
pub(crate) fn run_at(
    address: Option<&str>,
    req: &Request,
    parent: &str,
) -> Result<Vec<PathBuf>, Error> {
    let mut c = match address {
        Some(a) => Connection::open(a, CALL_TIMEOUT),
        None => Connection::session(CALL_TIMEOUT),
    }
    .map_err(unavailable)?;
    let token = token();
    let mut handle = request_path(&c.unique, &token);
    c.add_match(&match_rule(&handle), CALL_TIMEOUT)
        .map_err(unavailable)?;
    c.add_match(&owner_rule(), CALL_TIMEOUT)
        .map_err(unavailable)?;
    let reply = c
        .call(call_message(req, parent, &token), CALL_TIMEOUT)
        .map_err(unavailable)?;
    // Portals before version 0.9 ignore handle_token and pick their own path.
    if let Some(p) = reply.body.first().and_then(Value::as_str) {
        if p != handle {
            handle = p.to_string();
            c.add_match(&match_rule(&handle), CALL_TIMEOUT)
                .map_err(unavailable)?;
        }
    }
    let h = handle.clone();
    let m = c
        .wait(
            |m| {
                m.kind == dbus::SIGNAL
                    && ((m.path.as_deref() == Some(h.as_str())
                        && m.interface.as_deref() == Some(REQUEST_IFACE)
                        && m.member.as_deref() == Some("Response"))
                        || (m.member.as_deref() == Some("NameOwnerChanged")
                            && m.body.first().and_then(Value::as_str) == Some(DEST)
                            && m.body.get(2).and_then(Value::as_str) == Some("")))
            },
            None,
        )
        .map_err(unavailable)?;
    if m.member.as_deref() == Some("NameOwnerChanged") {
        return Err(Error::Unavailable(
            "xdg-desktop-portal quit during the dialog".into(),
        ));
    }
    parse_response(&m.body)
}

/// The portal's parent window handle for the active window: `x11:<XID>`
/// on X11; empty on Wayland (FLTK has no xdg-foreign export) and when no
/// window is shown. Call on the UI thread.
pub(crate) fn parent_handle() -> String {
    #[cfg(target_os = "linux")]
    {
        use fltk::prelude::*;
        if let Some(w) = fltk::app::first_window().filter(|w| w.shown()) {
            if !super::wayland() {
                let xid = w.raw_handle() as usize;
                if xid != 0 {
                    return format!("x11:{xid:x}");
                }
            }
        }
    }
    String::new()
}

#[cfg(test)]
static TEST_BUS: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Tests: a fake bus address [`run`] uses instead of the session bus.
#[cfg(test)]
pub(crate) fn set_test_bus(address: Option<String>) {
    *TEST_BUS.lock().unwrap_or_else(|e| e.into_inner()) = address;
}

#[cfg(test)]
fn test_bus() -> Option<String> {
    TEST_BUS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[cfg(not(test))]
fn test_bus() -> Option<String> {
    None
}

/// A fake session bus with a fake FileChooser portal, for tests: one
/// client, answers `Hello`, `AddMatch` and `OpenFile`/`SaveFile`, then
/// sends `Response` with `code` and `uris`.
#[cfg(test)]
pub(crate) mod fake {
    use super::dbus::{self, Message, Value};
    use std::io::{Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::sync::mpsc;

    pub(crate) enum Answer {
        /// `Response(code, {uris})`.
        Respond { code: u32, uris: Vec<String> },
        /// Reply with a request path that is not the predicted one first.
        OldPortal { uris: Vec<String> },
        /// An error reply to the call.
        Error(&'static str),
        /// The portal leaves the bus during the dialog.
        Vanish,
    }

    pub(crate) struct Fake {
        pub address: String,
        /// The FileChooser call the fake received.
        pub calls: mpsc::Receiver<Message>,
        dir: PathBuf,
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn read_msg(s: &mut UnixStream) -> Option<Message> {
        let mut head = [0u8; 16];
        s.read_exact(&mut head).ok()?;
        let n = Message::length(&head).ok()?;
        let mut buf = vec![0u8; n];
        buf[..16].copy_from_slice(&head);
        s.read_exact(&mut buf[16..]).ok()?;
        Some(Message::decode(&buf).expect("client sent valid D-Bus"))
    }

    fn reply(to: &Message, body: Vec<Value>) -> Message {
        Message {
            kind: dbus::METHOD_RETURN,
            serial: to.serial + 1000,
            reply_serial: Some(to.serial),
            sender: Some("org.freedesktop.DBus".into()),
            body,
            ..Default::default()
        }
    }

    fn signal(path: &str, iface: &str, member: &str, body: Vec<Value>) -> Message {
        Message {
            kind: dbus::SIGNAL,
            serial: 9000,
            path: Some(path.into()),
            interface: Some(iface.into()),
            member: Some(member.into()),
            body,
            ..Default::default()
        }
    }

    fn response(code: u32, uris: &[String]) -> Vec<Value> {
        let uris = Value::Array(
            dbus::Type::Basic(b's'),
            uris.iter().map(|u| Value::str(u.clone())).collect(),
        );
        vec![
            Value::U32(code),
            Value::dict(vec![
                ("uris", uris),
                (
                    "choices",
                    Value::Array(
                        dbus::Type::Struct(vec![dbus::Type::Basic(b's'), dbus::Type::Basic(b's')]),
                        vec![],
                    ),
                ),
            ]),
        ]
    }

    pub(crate) fn start(tag: &str, answer: Answer) -> Fake {
        let dir =
            std::env::temp_dir().join(format!("fltk-omarchy-portal-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("bus");
        let listener = UnixListener::bind(&sock).unwrap();
        let (tx, calls) = mpsc::channel();
        std::thread::spawn(move || {
            let Ok((mut s, _)) = listener.accept() else {
                return;
            };
            // SASL: NUL, AUTH line, BEGIN line.
            let mut b = [0u8; 1];
            s.read_exact(&mut b).unwrap();
            assert_eq!(b[0], 0);
            let mut line = Vec::new();
            while !line.ends_with(b"\r\n") {
                s.read_exact(&mut b).unwrap();
                line.push(b[0]);
            }
            assert!(line.starts_with(b"AUTH EXTERNAL "));
            s.write_all(b"OK 0123456789abcdef0123456789abcdef\r\n")
                .unwrap();
            let mut begin = [0u8; 7];
            s.read_exact(&mut begin).unwrap();
            assert_eq!(&begin, b"BEGIN\r\n");
            let unique = ":1.42";
            while let Some(m) = read_msg(&mut s) {
                match m.member.as_deref() {
                    Some("Hello") => {
                        s.write_all(&reply(&m, vec![Value::str(unique)]).encode())
                            .unwrap();
                    }
                    Some("AddMatch") => s.write_all(&reply(&m, vec![]).encode()).unwrap(),
                    Some("OpenFile") | Some("SaveFile") => {
                        let opts = m.body.get(2).cloned().unwrap();
                        let token = opts
                            .get("handle_token")
                            .and_then(Value::as_str)
                            .unwrap()
                            .to_string();
                        let handle = super::request_path(unique, &token);
                        let _ = tx.send(m.clone());
                        match &answer {
                            Answer::Respond { code, uris } => {
                                s.write_all(&reply(&m, vec![Value::Path(handle.clone())]).encode())
                                    .unwrap();
                                // An unrelated signal first: it must be skipped.
                                s.write_all(
                                    &signal(
                                        "/other",
                                        super::REQUEST_IFACE,
                                        "Response",
                                        response(0, &[]),
                                    )
                                    .encode(),
                                )
                                .unwrap();
                                s.write_all(
                                    &signal(
                                        &handle,
                                        super::REQUEST_IFACE,
                                        "Response",
                                        response(*code, uris),
                                    )
                                    .encode(),
                                )
                                .unwrap();
                            }
                            Answer::OldPortal { uris } => {
                                let old = format!("{}/request/old/1", super::PATH);
                                s.write_all(&reply(&m, vec![Value::Path(old.clone())]).encode())
                                    .unwrap();
                                // Wait for the new match before answering.
                                let add = read_msg(&mut s).unwrap();
                                assert_eq!(add.member.as_deref(), Some("AddMatch"));
                                assert!(add.body[0].as_str().unwrap().contains(&old));
                                s.write_all(&reply(&add, vec![]).encode()).unwrap();
                                s.write_all(
                                    &signal(
                                        &old,
                                        super::REQUEST_IFACE,
                                        "Response",
                                        response(0, uris),
                                    )
                                    .encode(),
                                )
                                .unwrap();
                            }
                            Answer::Error(name) => {
                                let e = Message {
                                    kind: dbus::ERROR,
                                    serial: 77,
                                    reply_serial: Some(m.serial),
                                    error_name: Some((*name).into()),
                                    body: vec![Value::str("nobody home")],
                                    ..Default::default()
                                };
                                s.write_all(&e.encode()).unwrap();
                            }
                            Answer::Vanish => {
                                s.write_all(&reply(&m, vec![Value::Path(handle.clone())]).encode())
                                    .unwrap();
                                s.write_all(
                                    &signal(
                                        "/org/freedesktop/DBus",
                                        "org.freedesktop.DBus",
                                        "NameOwnerChanged",
                                        vec![
                                            Value::str(super::DEST),
                                            Value::str(":1.5"),
                                            Value::str(""),
                                        ],
                                    )
                                    .encode(),
                                )
                                .unwrap();
                            }
                        }
                    }
                    _ => {}
                }
            }
        });
        Fake {
            address: format!("unix:path={}", sock.display()),
            calls,
            dir,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{start, Answer};
    use super::*;

    fn opts(m: &Message) -> &Value {
        &m.body[2]
    }

    #[test]
    fn request_paths() {
        assert_eq!(
            request_path(":1.42", "tok"),
            "/org/freedesktop/portal/desktop/request/1_42/tok"
        );
        let t = token();
        assert!(
            t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "{t}"
        );
        assert_ne!(token(), token());
    }

    #[test]
    fn open_arguments() {
        let req = Request::open("Pick images")
            .multiple(true)
            .accept_label("Use")
            .folder("/home/me/Pictures")
            .filter(Filter::new("Images").glob("*.png").mime("image/jpeg"))
            .filter(Filter::new("Empty"));
        let m = call_message(&req, "x11:1a", "tok");
        assert_eq!(m.member.as_deref(), Some("OpenFile"));
        assert_eq!(m.destination.as_deref(), Some(DEST));
        assert_eq!(m.interface.as_deref(), Some(IFACE));
        assert_eq!(m.signature(), "ssa{sv}");
        assert_eq!(m.body[0].as_str(), Some("x11:1a"));
        assert_eq!(m.body[1].as_str(), Some("Pick images"));
        let o = opts(&m);
        assert_eq!(o.get("handle_token").and_then(Value::as_str), Some("tok"));
        assert_eq!(o.get("multiple"), Some(&Value::Bool(true)));
        assert_eq!(o.get("directory"), Some(&Value::Bool(false)));
        assert_eq!(o.get("modal"), Some(&Value::Bool(true)));
        assert_eq!(o.get("accept_label").and_then(Value::as_str), Some("Use"));
        assert_eq!(
            o.get("current_folder"),
            Some(&Value::bytestring(b"/home/me/Pictures"))
        );
        let f = o.get("filters").unwrap();
        assert_eq!(f.ty().sig(), "a(sa(us))");
        // The filter without patterns is left out.
        let Value::Array(_, fs) = f else { panic!() };
        assert_eq!(fs.len(), 1);
        assert_eq!(
            fs[0],
            Value::Struct(vec![
                Value::str("Images"),
                Value::Array(
                    Type::Struct(vec![Type::Basic(b'u'), Type::Basic(b's')]),
                    vec![
                        Value::Struct(vec![Value::U32(0), Value::str("*.png")]),
                        Value::Struct(vec![Value::U32(1), Value::str("image/jpeg")]),
                    ]
                ),
            ])
        );
        assert_eq!(o.get("current_filter"), Some(&fs[0]));
        // The whole call marshals and decodes back.
        assert_eq!(Message::decode(&m.encode()).unwrap(), m);
    }

    #[test]
    fn save_arguments() {
        let req = Request::save("Export").name("notes.md");
        let m = call_message(&req, "", "t");
        assert_eq!(m.member.as_deref(), Some("SaveFile"));
        let o = opts(&m);
        assert_eq!(
            o.get("current_name").and_then(Value::as_str),
            Some("notes.md")
        );
        assert_eq!(o.get("multiple"), None);
        assert_eq!(o.get("filters"), None);
        let req = Request::open("Folder").directory(true);
        let o = call_message(&req, "", "t").body[2].clone();
        assert_eq!(o.get("directory"), Some(&Value::Bool(true)));
    }

    fn uris(list: &[&str]) -> Vec<Value> {
        let a = Value::Array(
            Type::Basic(b's'),
            list.iter().map(|u| Value::str(*u)).collect(),
        );
        vec![
            Value::U32(0),
            Value::dict(vec![("uris", a), ("writable", Value::Bool(true))]),
        ]
    }

    #[test]
    fn responses() {
        assert_eq!(
            parse_response(&uris(&[
                "file:///home/me/a%20b.txt",
                "file://localhost/x/%C3%A9",
                "https://e.com/x"
            ])),
            Ok(vec![
                PathBuf::from("/home/me/a b.txt"),
                PathBuf::from("/x/é")
            ])
        );
        assert_eq!(
            parse_response(&[Value::U32(1), Value::dict(vec![])]),
            Err(Error::Cancelled)
        );
        assert!(matches!(
            parse_response(&[Value::U32(2), Value::dict(vec![])]),
            Err(Error::Unavailable(_))
        ));
        assert!(matches!(parse_response(&[]), Err(Error::Unavailable(_))));
        assert_eq!(parse_response(&uris(&[])), Err(Error::Cancelled));
        assert_eq!(
            parse_response(&[Value::U32(0), Value::dict(vec![])]),
            Err(Error::Cancelled)
        );
    }

    #[test]
    fn uris_to_paths() {
        assert_eq!(uri_to_path("file:///a"), Some("/a".into()));
        assert_eq!(uri_to_path("file:///100%"), Some("/100%".into()));
        assert_eq!(uri_to_path("file://host/a"), None);
        assert_eq!(uri_to_path("/a"), None);
        // Bytes that are not UTF-8 stay as they are.
        use std::os::unix::ffi::OsStrExt;
        assert_eq!(
            uri_to_path("file:///%FF").unwrap().as_os_str().as_bytes(),
            b"/\xff"
        );
    }

    #[test]
    fn fake_portal_open() {
        let f = start(
            "open",
            Answer::Respond {
                code: 0,
                uris: vec![
                    "file:///home/me/one.txt".into(),
                    "file:///home/me/two%23.txt".into(),
                ],
            },
        );
        let req = Request::open("Open").multiple(true);
        let r = run_at(Some(&f.address), &req, "");
        assert_eq!(
            r,
            Ok(vec![
                PathBuf::from("/home/me/one.txt"),
                PathBuf::from("/home/me/two#.txt")
            ])
        );
        let call = f.calls.recv().unwrap();
        assert_eq!(call.member.as_deref(), Some("OpenFile"));
        assert_eq!(call.body[2].get("multiple"), Some(&Value::Bool(true)));
    }

    #[test]
    fn fake_portal_save_cancel() {
        let f = start(
            "cancel",
            Answer::Respond {
                code: 1,
                uris: vec![],
            },
        );
        let r = run_at(Some(&f.address), &Request::save("Save").name("a.txt"), "");
        assert_eq!(r, Err(Error::Cancelled));
        assert_eq!(f.calls.recv().unwrap().member.as_deref(), Some("SaveFile"));
    }

    #[test]
    fn fake_portal_old_handle() {
        let f = start(
            "old",
            Answer::OldPortal {
                uris: vec!["file:///x".into()],
            },
        );
        assert_eq!(
            run_at(Some(&f.address), &Request::open("Open"), ""),
            Ok(vec![PathBuf::from("/x")])
        );
    }

    #[test]
    fn fake_portal_missing() {
        let f = start(
            "missing",
            Answer::Error("org.freedesktop.DBus.Error.ServiceUnknown"),
        );
        let r = run_at(Some(&f.address), &Request::open("Open"), "");
        let Err(Error::Unavailable(why)) = r else {
            panic!("{r:?}")
        };
        assert!(why.contains("no xdg-desktop-portal file chooser"), "{why}");
        assert!(why.contains("nobody home"), "{why}");
    }

    #[test]
    fn fake_portal_vanishes() {
        let f = start("vanish", Answer::Vanish);
        let r = run_at(Some(&f.address), &Request::open("Open"), "");
        assert!(
            matches!(r, Err(Error::Unavailable(ref w)) if w.contains("quit")),
            "{r:?}"
        );
    }

    /// Talks to the real session bus without opening a dialog: reads the
    /// FileChooser version property. `cargo test -- --ignored real_bus`.
    #[test]
    #[ignore]
    fn real_bus_reads_portal_version() {
        let mut c = Connection::session(CALL_TIMEOUT).expect("session bus");
        assert!(c.unique.starts_with(':'), "{}", c.unique);
        let m = Message::call(
            DEST,
            PATH,
            "org.freedesktop.DBus.Properties",
            "Get",
            vec![Value::str(IFACE), Value::str("version")],
        );
        let r = c.call(m, CALL_TIMEOUT).expect("FileChooser version");
        let v = r.body.first().and_then(Value::as_u32).expect("u version");
        eprintln!("FileChooser portal version {v} (we are {})", c.unique);
        assert!(v >= 1);
    }

    #[test]
    fn no_bus() {
        let r = run_at(
            Some("unix:path=/nonexistent/fltk-omarchy/bus"),
            &Request::open("Open"),
            "",
        );
        assert!(matches!(r, Err(Error::Unavailable(_))), "{r:?}");
        let r = run_at(Some("tcp:host=localhost"), &Request::open("Open"), "");
        assert!(matches!(r, Err(Error::Unavailable(_))), "{r:?}");
    }
}
