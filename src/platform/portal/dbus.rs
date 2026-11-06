//! A minimal D-Bus client for the session bus, with no dependencies: the
//! wire format (marshalling both byte orders), SASL `EXTERNAL`
//! authentication, method calls and signals. It is just enough for
//! xdg-desktop-portal requests, which need one connection that stays open
//! from the method call until the `Response` signal: the portal closes a
//! request when its caller disconnects, so a one-shot `gdbus call` or
//! `dbus-send` process cannot wait for the answer.
//!
//! Spec: <https://dbus.freedesktop.org/doc/dbus-specification.html>.

use std::collections::VecDeque;
use std::fmt;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Largest message accepted (the spec's limit is 128 MiB).
const MAX_MESSAGE: usize = 64 << 20;
/// Deepest nesting of containers accepted in a value.
const MAX_DEPTH: usize = 32;

pub(crate) const METHOD_CALL: u8 = 1;
pub(crate) const METHOD_RETURN: u8 = 2;
pub(crate) const ERROR: u8 = 3;
pub(crate) const SIGNAL: u8 = 4;

#[derive(Debug)]
pub(crate) enum Error {
    /// No session bus address, or the bus refused the connection.
    NoBus(String),
    Io(std::io::Error),
    /// Bytes that are not D-Bus, or not what was expected.
    Protocol(String),
    /// An error reply: its D-Bus name and message.
    Remote(String, String),
    /// No answer in time.
    Timeout,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoBus(s) => write!(f, "no session bus: {s}"),
            Error::Io(e) => write!(f, "D-Bus I/O: {e}"),
            Error::Protocol(s) => write!(f, "D-Bus protocol: {s}"),
            Error::Remote(n, m) if m.is_empty() => write!(f, "{n}"),
            Error::Remote(n, m) => write!(f, "{n}: {m}"),
            Error::Timeout => write!(f, "D-Bus: no reply in time"),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        if matches!(
            e.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        ) {
            Error::Timeout
        } else {
            Error::Io(e)
        }
    }
}

fn bad<T>(s: impl Into<String>) -> Result<T, Error> {
    Err(Error::Protocol(s.into()))
}

// ------------------------------------------------------------------ types

/// A complete D-Bus type.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Type {
    /// `y b n q i u x t d s o g h`.
    Basic(u8),
    Array(Box<Type>),
    Struct(Vec<Type>),
    /// `{kv}`, only as an array element.
    Entry(Box<Type>, Box<Type>),
    Variant,
}

impl Type {
    /// Parses a signature into its complete types.
    pub(crate) fn parse(sig: &str) -> Result<Vec<Type>, Error> {
        let b = sig.as_bytes();
        if b.len() > 255 {
            return bad("signature longer than 255");
        }
        let mut pos = 0;
        let mut out = Vec::new();
        while pos < b.len() {
            out.push(Type::one(b, &mut pos, 0)?);
        }
        Ok(out)
    }

    fn one(b: &[u8], pos: &mut usize, depth: usize) -> Result<Type, Error> {
        if depth > MAX_DEPTH {
            return bad("signature nested too deeply");
        }
        let Some(&c) = b.get(*pos) else {
            return bad("signature ends early");
        };
        *pos += 1;
        Ok(match c {
            b'y' | b'b' | b'n' | b'q' | b'i' | b'u' | b'x' | b't' | b'd' | b's' | b'o' | b'g'
            | b'h' => Type::Basic(c),
            b'v' => Type::Variant,
            b'a' => {
                if b.get(*pos) == Some(&b'{') {
                    *pos += 1;
                    let k = Type::one(b, pos, depth + 1)?;
                    if !matches!(k, Type::Basic(_)) {
                        return bad("dict key is not a basic type");
                    }
                    let v = Type::one(b, pos, depth + 1)?;
                    if b.get(*pos) != Some(&b'}') {
                        return bad("dict entry not closed");
                    }
                    *pos += 1;
                    Type::Array(Box::new(Type::Entry(Box::new(k), Box::new(v))))
                } else {
                    Type::Array(Box::new(Type::one(b, pos, depth + 1)?))
                }
            }
            b'(' => {
                let mut items = Vec::new();
                while b.get(*pos) != Some(&b')') {
                    items.push(Type::one(b, pos, depth + 1)?);
                }
                *pos += 1;
                if items.is_empty() {
                    return bad("empty struct");
                }
                Type::Struct(items)
            }
            _ => return bad(format!("unknown type code {:?}", c as char)),
        })
    }

    /// The single complete type of `sig`.
    pub(crate) fn single(sig: &str) -> Result<Type, Error> {
        let mut t = Type::parse(sig)?;
        if t.len() != 1 {
            return bad(format!("{sig:?} is not one complete type"));
        }
        Ok(t.remove(0))
    }

    pub(crate) fn sig(&self) -> String {
        let mut s = String::new();
        self.write_sig(&mut s);
        s
    }

    fn write_sig(&self, s: &mut String) {
        match self {
            Type::Basic(c) => s.push(*c as char),
            Type::Variant => s.push('v'),
            Type::Array(t) => {
                s.push('a');
                t.write_sig(s);
            }
            Type::Struct(items) => {
                s.push('(');
                items.iter().for_each(|t| t.write_sig(s));
                s.push(')');
            }
            Type::Entry(k, v) => {
                s.push('{');
                k.write_sig(s);
                v.write_sig(s);
                s.push('}');
            }
        }
    }

    fn align(&self) -> usize {
        match self {
            Type::Basic(b'y') | Type::Basic(b'g') | Type::Variant => 1,
            Type::Basic(b'n') | Type::Basic(b'q') => 2,
            Type::Basic(b'x') | Type::Basic(b't') | Type::Basic(b'd') => 8,
            Type::Struct(_) | Type::Entry(..) => 8,
            _ => 4,
        }
    }
}

// ----------------------------------------------------------------- values

/// A D-Bus value.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Value {
    Byte(u8),
    Bool(bool),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    F64(f64),
    Str(String),
    Path(String),
    Sig(String),
    /// Unix file descriptor index (never sent by this client).
    Fd(u32),
    /// The element type and the elements.
    Array(Type, Vec<Value>),
    Struct(Vec<Value>),
    Entry(Box<Value>, Box<Value>),
    Variant(Box<Value>),
}

impl Value {
    pub(crate) fn ty(&self) -> Type {
        match self {
            Value::Byte(_) => Type::Basic(b'y'),
            Value::Bool(_) => Type::Basic(b'b'),
            Value::I16(_) => Type::Basic(b'n'),
            Value::U16(_) => Type::Basic(b'q'),
            Value::I32(_) => Type::Basic(b'i'),
            Value::U32(_) => Type::Basic(b'u'),
            Value::I64(_) => Type::Basic(b'x'),
            Value::U64(_) => Type::Basic(b't'),
            Value::F64(_) => Type::Basic(b'd'),
            Value::Str(_) => Type::Basic(b's'),
            Value::Path(_) => Type::Basic(b'o'),
            Value::Sig(_) => Type::Basic(b'g'),
            Value::Fd(_) => Type::Basic(b'h'),
            Value::Array(t, _) => Type::Array(Box::new(t.clone())),
            Value::Struct(items) => Type::Struct(items.iter().map(Value::ty).collect()),
            Value::Entry(k, v) => Type::Entry(Box::new(k.ty()), Box::new(v.ty())),
            Value::Variant(_) => Type::Variant,
        }
    }

    pub(crate) fn str(s: impl Into<String>) -> Value {
        Value::Str(s.into())
    }

    /// `a{sv}` from `(key, value)` pairs (each value is wrapped in a
    /// variant).
    pub(crate) fn dict(entries: Vec<(&str, Value)>) -> Value {
        let t = Type::Entry(Box::new(Type::Basic(b's')), Box::new(Type::Variant));
        Value::Array(
            t,
            entries
                .into_iter()
                .map(|(k, v)| {
                    Value::Entry(
                        Box::new(Value::str(k)),
                        Box::new(Value::Variant(Box::new(v))),
                    )
                })
                .collect(),
        )
    }

    /// `ay` holding a NUL-terminated byte string (portal paths).
    pub(crate) fn bytestring(b: &[u8]) -> Value {
        let mut v: Vec<Value> = b.iter().map(|&c| Value::Byte(c)).collect();
        v.push(Value::Byte(0));
        Value::Array(Type::Basic(b'y'), v)
    }

    /// The value inside any number of variants.
    pub(crate) fn unwrap_variant(&self) -> &Value {
        let mut v = self;
        while let Value::Variant(inner) = v {
            v = inner;
        }
        v
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        match self.unwrap_variant() {
            Value::Str(s) | Value::Path(s) | Value::Sig(s) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn as_u32(&self) -> Option<u32> {
        match self.unwrap_variant() {
            Value::U32(n) => Some(*n),
            _ => None,
        }
    }

    /// The entry with key `key` of an `a{s*}` dictionary.
    pub(crate) fn get(&self, key: &str) -> Option<&Value> {
        let Value::Array(_, items) = self.unwrap_variant() else {
            return None;
        };
        items.iter().find_map(|e| match e {
            Value::Entry(k, v) if k.as_str() == Some(key) => Some(v.unwrap_variant()),
            _ => None,
        })
    }
}

// ------------------------------------------------------------- marshalling

/// Little-endian writer; offsets are relative to the start of the buffer,
/// which is 8-aligned in the message (header, body).
#[derive(Default)]
struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    fn pad(&mut self, a: usize) {
        while self.buf.len() % a != 0 {
            self.buf.push(0);
        }
    }

    fn u32(&mut self, n: u32) {
        self.pad(4);
        self.buf.extend_from_slice(&n.to_le_bytes());
    }

    fn put(&mut self, v: &Value) {
        match v {
            Value::Byte(b) => self.buf.push(*b),
            Value::Bool(b) => self.u32(*b as u32),
            Value::I16(n) => {
                self.pad(2);
                self.buf.extend_from_slice(&n.to_le_bytes());
            }
            Value::U16(n) => {
                self.pad(2);
                self.buf.extend_from_slice(&n.to_le_bytes());
            }
            Value::I32(n) => self.u32(*n as u32),
            Value::U32(n) | Value::Fd(n) => self.u32(*n),
            Value::I64(n) => {
                self.pad(8);
                self.buf.extend_from_slice(&n.to_le_bytes());
            }
            Value::U64(n) => {
                self.pad(8);
                self.buf.extend_from_slice(&n.to_le_bytes());
            }
            Value::F64(n) => {
                self.pad(8);
                self.buf.extend_from_slice(&n.to_le_bytes());
            }
            Value::Str(s) | Value::Path(s) => {
                self.u32(s.len() as u32);
                self.buf.extend_from_slice(s.as_bytes());
                self.buf.push(0);
            }
            Value::Sig(s) => {
                self.buf.push(s.len() as u8);
                self.buf.extend_from_slice(s.as_bytes());
                self.buf.push(0);
            }
            Value::Array(t, items) => {
                self.u32(0);
                let at = self.buf.len() - 4;
                // The padding to the first element is not part of the length.
                self.pad(t.align());
                let start = self.buf.len();
                items.iter().for_each(|i| self.put(i));
                let n = (self.buf.len() - start) as u32;
                self.buf[at..at + 4].copy_from_slice(&n.to_le_bytes());
            }
            Value::Struct(items) => {
                self.pad(8);
                items.iter().for_each(|i| self.put(i));
            }
            Value::Entry(k, v) => {
                self.pad(8);
                self.put(k);
                self.put(v);
            }
            Value::Variant(inner) => {
                self.put(&Value::Sig(inner.ty().sig()));
                self.put(inner);
            }
        }
    }
}

/// Reader for either byte order.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    big: bool,
}

impl<'a> Reader<'a> {
    fn align(&mut self, a: usize) -> Result<(), Error> {
        let p = self.pos.div_ceil(a) * a;
        if p > self.buf.len() {
            return bad("value runs past the end");
        }
        self.pos = p;
        Ok(())
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.buf.len());
        let Some(end) = end else {
            return bad("value runs past the end");
        };
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn fixed<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        self.align(N)?;
        let mut a = [0u8; N];
        a.copy_from_slice(self.take(N)?);
        if self.big {
            a.reverse();
        }
        Ok(a) // now little-endian
    }

    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.fixed::<4>()?))
    }

    fn string(&mut self, len: usize) -> Result<String, Error> {
        let b = self.take(len)?;
        if self.take(1)? != [0] {
            return bad("string not NUL-terminated");
        }
        String::from_utf8(b.to_vec()).or_else(|_| bad("string is not UTF-8"))
    }

    fn get(&mut self, t: &Type, depth: usize) -> Result<Value, Error> {
        if depth > MAX_DEPTH {
            return bad("value nested too deeply");
        }
        Ok(match t {
            Type::Basic(c) => match c {
                b'y' => Value::Byte(self.take(1)?[0]),
                b'b' => match self.u32()? {
                    0 => Value::Bool(false),
                    1 => Value::Bool(true),
                    _ => return bad("boolean is not 0 or 1"),
                },
                b'n' => Value::I16(i16::from_le_bytes(self.fixed::<2>()?)),
                b'q' => Value::U16(u16::from_le_bytes(self.fixed::<2>()?)),
                b'i' => Value::I32(i32::from_le_bytes(self.fixed::<4>()?)),
                b'u' => Value::U32(self.u32()?),
                b'h' => Value::Fd(self.u32()?),
                b'x' => Value::I64(i64::from_le_bytes(self.fixed::<8>()?)),
                b't' => Value::U64(u64::from_le_bytes(self.fixed::<8>()?)),
                b'd' => Value::F64(f64::from_le_bytes(self.fixed::<8>()?)),
                b's' | b'o' => {
                    let n = self.u32()? as usize;
                    let s = self.string(n)?;
                    if *c == b's' {
                        Value::Str(s)
                    } else {
                        Value::Path(s)
                    }
                }
                b'g' => {
                    let n = self.take(1)?[0] as usize;
                    Value::Sig(self.string(n)?)
                }
                _ => return bad("unknown basic type"),
            },
            Type::Array(e) => {
                let n = self.u32()? as usize;
                if n > MAX_MESSAGE {
                    return bad("array too long");
                }
                self.align(e.align())?;
                let end = self.pos + n;
                if end > self.buf.len() {
                    return bad("array runs past the end");
                }
                let mut items = Vec::new();
                while self.pos < end {
                    items.push(self.get(e, depth + 1)?);
                }
                if self.pos != end {
                    return bad("array length does not match its elements");
                }
                Value::Array((**e).clone(), items)
            }
            Type::Struct(items) => {
                self.align(8)?;
                let mut v = Vec::with_capacity(items.len());
                for t in items {
                    v.push(self.get(t, depth + 1)?);
                }
                Value::Struct(v)
            }
            Type::Entry(k, v) => {
                self.align(8)?;
                let k = self.get(k, depth + 1)?;
                let v = self.get(v, depth + 1)?;
                Value::Entry(Box::new(k), Box::new(v))
            }
            Type::Variant => {
                let n = self.take(1)?[0] as usize;
                let sig = self.string(n)?;
                let t = Type::single(&sig)?;
                Value::Variant(Box::new(self.get(&t, depth + 1)?))
            }
        })
    }
}

/// Marshals `values` little-endian from an 8-aligned offset.
pub(crate) fn marshal(values: &[Value]) -> Vec<u8> {
    let mut w = Writer::default();
    values.iter().for_each(|v| w.put(v));
    w.buf
}

/// Reads values of signature `sig` from `buf` (8-aligned start).
pub(crate) fn unmarshal(buf: &[u8], sig: &str, big: bool) -> Result<Vec<Value>, Error> {
    let mut r = Reader { buf, pos: 0, big };
    let mut out = Vec::new();
    for t in Type::parse(sig)? {
        out.push(r.get(&t, 0)?);
    }
    if r.pos != buf.len() {
        return bad("trailing bytes after the values");
    }
    Ok(out)
}

// ---------------------------------------------------------------- messages

/// A D-Bus message.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Message {
    pub kind: u8,
    pub flags: u8,
    pub serial: u32,
    pub path: Option<String>,
    pub interface: Option<String>,
    pub member: Option<String>,
    pub error_name: Option<String>,
    pub reply_serial: Option<u32>,
    pub destination: Option<String>,
    pub sender: Option<String>,
    pub body: Vec<Value>,
}

impl Message {
    pub(crate) fn call(
        dest: &str,
        path: &str,
        iface: &str,
        member: &str,
        body: Vec<Value>,
    ) -> Self {
        Message {
            kind: METHOD_CALL,
            path: Some(path.into()),
            interface: Some(iface.into()),
            member: Some(member.into()),
            destination: Some(dest.into()),
            body,
            ..Default::default()
        }
    }

    pub(crate) fn signature(&self) -> String {
        self.body.iter().map(|v| v.ty().sig()).collect()
    }

    /// Wire bytes, little-endian.
    pub(crate) fn encode(&self) -> Vec<u8> {
        let body = marshal(&self.body);
        let mut fields = Vec::new();
        let mut field = |code: u8, v: Value| {
            fields.push(Value::Struct(vec![
                Value::Byte(code),
                Value::Variant(Box::new(v)),
            ]));
        };
        if let Some(p) = &self.path {
            field(1, Value::Path(p.clone()));
        }
        if let Some(s) = &self.interface {
            field(2, Value::str(s.clone()));
        }
        if let Some(s) = &self.member {
            field(3, Value::str(s.clone()));
        }
        if let Some(s) = &self.error_name {
            field(4, Value::str(s.clone()));
        }
        if let Some(n) = self.reply_serial {
            field(5, Value::U32(n));
        }
        if let Some(s) = &self.destination {
            field(6, Value::str(s.clone()));
        }
        if let Some(s) = &self.sender {
            field(7, Value::str(s.clone()));
        }
        let sig = self.signature();
        if !sig.is_empty() {
            field(8, Value::Sig(sig));
        }
        let header_fields =
            Value::Array(Type::Struct(vec![Type::Basic(b'y'), Type::Variant]), fields);
        let mut w = Writer::default();
        w.buf.extend_from_slice(&[b'l', self.kind, self.flags, 1]);
        w.u32(body.len() as u32);
        w.u32(self.serial);
        w.put(&header_fields);
        w.pad(8);
        w.buf.extend_from_slice(&body);
        w.buf
    }

    /// Total length of a message from its first 16 bytes.
    pub(crate) fn length(head: &[u8; 16]) -> Result<usize, Error> {
        let big = match head[0] {
            b'l' => false,
            b'B' => true,
            _ => return bad("unknown byte order"),
        };
        let word = |i: usize| {
            let b = [head[i], head[i + 1], head[i + 2], head[i + 3]];
            if big {
                u32::from_be_bytes(b)
            } else {
                u32::from_le_bytes(b)
            }
        };
        let (body, fields) = (word(4) as usize, word(12) as usize);
        let total = (16 + fields).div_ceil(8) * 8 + body;
        if total > MAX_MESSAGE {
            return bad("message too long");
        }
        Ok(total)
    }

    pub(crate) fn decode(buf: &[u8]) -> Result<Message, Error> {
        if buf.len() < 16 {
            return bad("message shorter than its header");
        }
        let big = match buf[0] {
            b'l' => false,
            b'B' => true,
            _ => return bad("unknown byte order"),
        };
        let mut r = Reader { buf, pos: 0, big };
        let fixed = r.take(4)?;
        let (kind, flags) = (fixed[1], fixed[2]);
        if fixed[3] != 1 {
            return bad("unknown protocol version");
        }
        let body_len = r.u32()? as usize;
        let serial = r.u32()?;
        let ft = Type::Array(Box::new(Type::Struct(vec![
            Type::Basic(b'y'),
            Type::Variant,
        ])));
        let Value::Array(_, fields) = r.get(&ft, 0)? else {
            return bad("header fields");
        };
        r.align(8)?;
        let body = r.take(body_len)?;
        if r.pos != buf.len() {
            return bad("bytes after the body");
        }
        let mut m = Message {
            kind,
            flags,
            serial,
            ..Default::default()
        };
        let mut sig = String::new();
        for f in &fields {
            let Value::Struct(f) = f else { continue };
            let (Some(Value::Byte(code)), Some(v)) = (f.first(), f.get(1)) else {
                continue;
            };
            let v = v.unwrap_variant();
            let s = v.as_str().map(str::to_string);
            match code {
                1 => m.path = s,
                2 => m.interface = s,
                3 => m.member = s,
                4 => m.error_name = s,
                5 => m.reply_serial = v.as_u32(),
                6 => m.destination = s,
                7 => m.sender = s,
                8 => sig = s.unwrap_or_default(),
                _ => {}
            }
        }
        m.body = unmarshal(body, &sig, big)?;
        Ok(m)
    }

    /// The error reply's name and its message (first string argument).
    fn remote_error(&self) -> Error {
        Error::Remote(
            self.error_name.clone().unwrap_or_default(),
            self.body
                .first()
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        )
    }
}

// --------------------------------------------------------------- transport

/// Where the session bus listens: `unix:path=` entries of
/// `DBUS_SESSION_BUS_ADDRESS` (and `unix:abstract=` on Linux), else
/// `$XDG_RUNTIME_DIR/bus`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Address {
    Path(PathBuf),
    Abstract(Vec<u8>),
}

/// Parses a D-Bus server address list (`a:k=v,k=v;b:...`), keeping the
/// Unix-socket entries this client can use, in order.
pub(crate) fn parse_address(list: &str) -> Vec<Address> {
    let mut out = Vec::new();
    for entry in list.split(';') {
        let Some(("unix", params)) = entry.split_once(':') else {
            continue;
        };
        for kv in params.split(',') {
            match kv.split_once('=') {
                Some(("path", v)) => {
                    out.push(Address::Path(PathBuf::from(os_string(unescape(v)))));
                }
                Some(("abstract", v)) => out.push(Address::Abstract(unescape(v))),
                _ => {}
            }
        }
    }
    out
}

fn os_string(b: Vec<u8>) -> std::ffi::OsString {
    use std::os::unix::ffi::OsStringExt;
    std::ffi::OsString::from_vec(b)
}

/// D-Bus address values escape bytes as `%xx`.
fn unescape(v: &str) -> Vec<u8> {
    let b = v.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
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
    out
}

fn session_addresses() -> Vec<Address> {
    let mut out = std::env::var("DBUS_SESSION_BUS_ADDRESS")
        .map(|a| parse_address(&a))
        .unwrap_or_default();
    if out.is_empty() {
        if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
            let p = PathBuf::from(dir).join("bus");
            if p.exists() {
                out.push(Address::Path(p));
            }
        }
    }
    out
}

fn connect_to(a: &Address) -> std::io::Result<UnixStream> {
    match a {
        Address::Path(p) => UnixStream::connect(p),
        Address::Abstract(name) => connect_abstract(name),
    }
}

#[cfg(target_os = "linux")]
fn connect_abstract(name: &[u8]) -> std::io::Result<UnixStream> {
    use std::os::linux::net::SocketAddrExt;
    let a = std::os::unix::net::SocketAddr::from_abstract_name(name)?;
    UnixStream::connect_addr(&a)
}

#[cfg(not(target_os = "linux"))]
fn connect_abstract(_: &[u8]) -> std::io::Result<UnixStream> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "abstract sockets are Linux-only",
    ))
}

/// One authenticated connection to a bus.
pub(crate) struct Connection {
    stream: UnixStream,
    serial: u32,
    /// Our unique bus name (`:1.42`), from `Hello`.
    pub unique: String,
    /// Messages read while waiting for something else.
    queue: VecDeque<Message>,
}

impl Connection {
    /// Connects to the session bus.
    pub(crate) fn session(timeout: Duration) -> Result<Connection, Error> {
        let list = session_addresses();
        if list.is_empty() {
            return Err(Error::NoBus(
                "DBUS_SESSION_BUS_ADDRESS has no usable unix: address".into(),
            ));
        }
        let mut last = String::new();
        for a in &list {
            match connect_to(a) {
                Ok(s) => return Connection::start(s, timeout),
                Err(e) => last = format!("{a:?}: {e}"),
            }
        }
        Err(Error::NoBus(last))
    }

    /// Connects to `address` (a D-Bus address list).
    pub(crate) fn open(address: &str, timeout: Duration) -> Result<Connection, Error> {
        let list = parse_address(address);
        let a = list
            .first()
            .ok_or_else(|| Error::NoBus(format!("no usable address in {address:?}")))?;
        let s = connect_to(a).map_err(|e| Error::NoBus(format!("{a:?}: {e}")))?;
        Connection::start(s, timeout)
    }

    fn start(stream: UnixStream, timeout: Duration) -> Result<Connection, Error> {
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        let mut c = Connection {
            stream,
            serial: 0,
            unique: String::new(),
            queue: VecDeque::new(),
        };
        c.auth()?;
        let reply = c.call(
            Message::call(
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "Hello",
                vec![],
            ),
            timeout,
        )?;
        c.unique = reply
            .body
            .first()
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into();
        if c.unique.is_empty() {
            return bad("Hello gave no unique name");
        }
        Ok(c)
    }

    fn auth(&mut self) -> Result<(), Error> {
        // SAFETY: getuid cannot fail.
        let uid = unsafe { libc::getuid() }.to_string();
        let hex: String = uid.bytes().map(|b| format!("{b:02x}")).collect();
        self.stream.write_all(b"\0")?;
        self.stream
            .write_all(format!("AUTH EXTERNAL {hex}\r\n").as_bytes())?;
        let line = self.line()?;
        if !line.starts_with("OK ") {
            return Err(Error::NoBus(format!("authentication refused: {line}")));
        }
        self.stream.write_all(b"BEGIN\r\n")?;
        Ok(())
    }

    fn line(&mut self) -> Result<String, Error> {
        let mut out = Vec::new();
        let mut b = [0u8; 1];
        while !out.ends_with(b"\r\n") {
            if out.len() > 4096 {
                return bad("authentication line too long");
            }
            if self.stream.read(&mut b)? == 0 {
                return Err(Error::NoBus("the bus closed the connection".into()));
            }
            out.push(b[0]);
        }
        out.truncate(out.len() - 2);
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    /// Sends `m` with the next serial; returns the serial.
    pub(crate) fn send(&mut self, mut m: Message) -> Result<u32, Error> {
        self.serial = self.serial.wrapping_add(1).max(1);
        m.serial = self.serial;
        self.stream.write_all(&m.encode())?;
        Ok(m.serial)
    }

    /// Reads the next message from the socket (`None` timeout: wait).
    fn read(&mut self, deadline: Option<Instant>) -> Result<Message, Error> {
        let left = match deadline {
            Some(d) => {
                let l = d.saturating_duration_since(Instant::now());
                if l.is_zero() {
                    return Err(Error::Timeout);
                }
                Some(l)
            }
            None => None,
        };
        self.stream.set_read_timeout(left)?;
        let mut head = [0u8; 16];
        self.stream.read_exact(&mut head)?;
        let n = Message::length(&head)?;
        let mut buf = vec![0u8; n];
        buf[..16].copy_from_slice(&head);
        self.stream.read_exact(&mut buf[16..])?;
        Message::decode(&buf)
    }

    /// Calls a method and waits for its reply; other messages are kept for
    /// [`Connection::wait`]. An error reply is `Err(Error::Remote)`.
    pub(crate) fn call(&mut self, m: Message, timeout: Duration) -> Result<Message, Error> {
        let serial = self.send(m)?;
        let deadline = Instant::now() + timeout;
        loop {
            let r = self.read(Some(deadline))?;
            if r.reply_serial == Some(serial) && (r.kind == METHOD_RETURN || r.kind == ERROR) {
                if r.kind == ERROR {
                    return Err(r.remote_error());
                }
                return Ok(r);
            }
            self.queue.push_back(r);
        }
    }

    /// Waits for a message `pick` accepts (queued ones first); `None`
    /// timeout waits as long as it takes.
    pub(crate) fn wait(
        &mut self,
        mut pick: impl FnMut(&Message) -> bool,
        timeout: Option<Duration>,
    ) -> Result<Message, Error> {
        if let Some(i) = self.queue.iter().position(&mut pick) {
            return Ok(self.queue.remove(i).expect("index from position"));
        }
        let deadline = timeout.map(|t| Instant::now() + t);
        loop {
            let m = self.read(deadline)?;
            if pick(&m) {
                return Ok(m);
            }
        }
    }

    /// `org.freedesktop.DBus.AddMatch(rule)`.
    pub(crate) fn add_match(&mut self, rule: &str, timeout: Duration) -> Result<(), Error> {
        self.call(
            Message::call(
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "AddMatch",
                vec![Value::str(rule)],
            ),
            timeout,
        )
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_round_trip() {
        for s in [
            "s", "a{sv}", "(sa(us))", "aay", "ua{sv}", "a(ss)", "v", "a{s(ii)}",
        ] {
            let t = Type::parse(s).unwrap();
            assert_eq!(t.iter().map(Type::sig).collect::<String>(), s);
        }
        assert!(Type::parse("a{vs}").is_err());
        assert!(Type::parse("(").is_err());
        assert!(Type::parse("()").is_err());
        assert!(Type::parse("z").is_err());
        assert!(Type::single("ss").is_err());
    }

    #[test]
    fn values_round_trip_with_alignment() {
        let v = vec![
            Value::Byte(7),
            Value::str("hi"),
            Value::dict(vec![
                ("multiple", Value::Bool(true)),
                ("n", Value::U64(1 << 40)),
                ("f", Value::bytestring(b"/tmp")),
            ]),
            Value::Struct(vec![Value::I16(-2), Value::F64(1.5)]),
        ];
        let b = marshal(&v);
        let sig: String = v.iter().map(|x| x.ty().sig()).collect();
        assert_eq!(sig, "ysa{sv}(nd)");
        assert_eq!(unmarshal(&b, &sig, false).unwrap(), v);
    }

    #[test]
    fn known_bytes() {
        // "s" = "ab": length 2, bytes, NUL.
        assert_eq!(marshal(&[Value::str("ab")]), b"\x02\0\0\0ab\0");
        // ay [1]: length 1, no padding for bytes.
        assert_eq!(
            marshal(&[Value::Array(Type::Basic(b'y'), vec![Value::Byte(1)])]),
            b"\x01\0\0\0\x01"
        );
        // a(u) with one element: length 4, then padding to 8 not counted.
        let a = Value::Array(
            Type::Struct(vec![Type::Basic(b'u')]),
            vec![Value::Struct(vec![Value::U32(9)])],
        );
        assert_eq!(marshal(&[a]), b"\x04\0\0\0\0\0\0\0\x09\0\0\0");
        // An empty array of structs still pads to the element alignment.
        let e = Value::Array(Type::Struct(vec![Type::Basic(b'u')]), vec![]);
        assert_eq!(marshal(&[e]), b"\0\0\0\0\0\0\0\0");
    }

    #[test]
    fn big_endian_values() {
        let b = [0, 0, 0, 2, b'a', b'b', 0, 0, 0, 0, 0, 5];
        assert_eq!(
            unmarshal(&b, "su", true).unwrap(),
            vec![Value::str("ab"), Value::U32(5)]
        );
    }

    #[test]
    fn rejects_bad_bytes() {
        assert!(unmarshal(b"\x05\0\0\0ab\0", "s", false).is_err()); // past the end
        assert!(unmarshal(b"\x02\0\0\0abX", "s", false).is_err()); // no NUL
        assert!(unmarshal(b"\x02\0\0\0", "b", false).is_err()); // bool 2
        assert!(unmarshal(b"\x01\0\0\0\x01\0", "ay", false).is_err()); // trailing
        assert!(unmarshal(b"\x01v\0", "v", false).is_err()); // a variant whose value is missing
    }

    #[test]
    fn messages_round_trip() {
        let mut m = Message::call(
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.FileChooser",
            "OpenFile",
            vec![
                Value::str(""),
                Value::str("Open"),
                Value::dict(vec![("modal", Value::Bool(true))]),
            ],
        );
        m.serial = 3;
        let b = m.encode();
        let mut head = [0u8; 16];
        head.copy_from_slice(&b[..16]);
        assert_eq!(Message::length(&head).unwrap(), b.len());
        let d = Message::decode(&b).unwrap();
        assert_eq!(d, m);
        assert_eq!(d.signature(), "ssa{sv}");
    }

    #[test]
    fn addresses() {
        assert_eq!(
            parse_address("unix:path=/run/user/1000/bus"),
            vec![Address::Path("/run/user/1000/bus".into())]
        );
        assert_eq!(
            parse_address(
                "tcp:host=x,port=1;unix:abstract=/tmp/dbus-AB%2c,guid=12;unix:path=/a%20b"
            ),
            vec![
                Address::Abstract(b"/tmp/dbus-AB,".to_vec()),
                Address::Path("/a b".into())
            ]
        );
        assert!(parse_address("unix:tmpdir=/tmp").is_empty());
        assert!(parse_address("").is_empty());
        assert_eq!(unescape("a%"), b"a%");
        assert_eq!(unescape("a%4"), b"a%4");
        assert_eq!(unescape("%41"), b"A");
    }
}
