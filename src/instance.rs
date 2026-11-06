//! Single instance and app IPC: a second launch, or a CLI such as
//! `myapp call toggle`, reaches the running app.
//!
//! ```no_run
//! use fltk_omarchy::{instance, App, AppWindow};
//! // Before App::new: a second launch forwards its arguments to the
//! // running app ("activate") and exits here.
//! let server = instance::acquire_or_forward("dev.you.app");
//! let app = App::new("dev.you.app");
//! app.keep_running(true); // stays alive while the window is hidden
//! let mut win = AppWindow::new("You", 640, 480);
//! win.end();
//! server.manage(&win); // show, hide, toggle, raise, window, activate
//! server.handle("state", |_| r#"{"ok":true}"#.to_string());
//! if !std::env::args().any(|a| a == "--hidden") {
//!     win.show();
//! }
//! app.run();
//! ```
//!
//! From another process (a CLI, a bar widget):
//! `instance::call("dev.you.app", "toggle", "")` returns the reply string.
//!
//! **Protocol.** One request per connection: a function name, string
//! arguments and the caller's working directory; the reply is a string
//! (JSON by convention) or an error. Frames are a 4-byte big-endian length
//! and UTF-8 JSON (`{"v":1,"fn":..,"args":[..],"cwd":..}`, replies
//! `{"ok":..}` or `{"err":..}`). Requests are at most [`MAX_REQUEST`]
//! bytes and replies [`MAX_REPLY`]; reads and writes time out
//! ([`Options::io_timeout`]); at most [`Options::max_connections`]
//! requests are served at once.
//!
//! **Threads.** The server thread hands each request to the UI thread
//! ([`crate::task::post`]) and waits for the handler's reply (at most
//! [`Options::handler_timeout`]). Handlers run on the UI thread, like any
//! callback, and are registered there ([`Server::handle`]). Call
//! [`acquire`] on the UI (main) thread.
//!
//! **Built-in functions** (an app handler with the same name replaces
//! one): `ping` (`{"pid":..,"app":..}`), `functions` (JSON list),
//! `activate` (a second launch: [`Server::on_activate`], then raises the
//! managed window), `quit` ([`App::quit`](crate::App::quit)); with
//! [`Server::manage`]: `show`, `raise`, `hide`, `toggle` (`{"shown":..}`),
//! `window` (`{"shown":..,"active":..}`) and `is_active` (`true`/`false`).
//!
//! **Per OS** (transport in `platform::instance`): Unix socket in a 0700
//! directory of [`dirs::runtime`](crate::platform::dirs::runtime) with an
//! `flock` lock on Linux, the BSDs and macOS (peer uid checked on both
//! ends); a named pipe on Windows. Where none is available, or the
//! endpoint cannot be used, [`acquire`] returns a primary without IPC and
//! says so on stderr ([`Server::unavailable`]). docs/PLATFORMS.md.

use crate::platform::instance::{self as ipc, Claim, Conn};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::io::{self, Read, Write};
use std::marker::PhantomData;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

/// Largest request a server accepts (1 MiB).
pub const MAX_REQUEST: usize = 1 << 20;
/// Largest reply a server sends and a client reads (8 MiB).
pub const MAX_REPLY: usize = 8 << 20;
const VERSION: u64 = 1;

// ------------------------------------------------------------------ errors

/// Why a call failed.
#[derive(Debug)]
pub enum Error {
    /// No instance of the app is running.
    NotRunning,
    /// No reply in time (the client's timeout).
    Timeout,
    /// The request or the reply is over its size limit.
    TooLarge,
    /// The app answered with an error (unknown function, a handler's
    /// `Err`, the server's own timeout, too many connections…).
    Remote(String),
    /// Malformed data on the connection.
    Protocol(String),
    /// No local IPC here (OS, runtime directory, another user's endpoint).
    Unavailable(String),
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotRunning => write!(f, "the app is not running"),
            Error::Timeout => write!(f, "the app did not answer in time"),
            Error::TooLarge => write!(f, "message too large"),
            Error::Remote(e) => write!(f, "{e}"),
            Error::Protocol(e) => write!(f, "protocol error: {e}"),
            Error::Unavailable(e) => write!(f, "IPC unavailable: {e}"),
            Error::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

fn io_error(e: io::Error) -> Error {
    match e.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => Error::Timeout,
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => Error::NotRunning,
        io::ErrorKind::PermissionDenied | io::ErrorKind::Unsupported => Error::Unavailable(e.to_string()),
        _ => Error::Io(e),
    }
}

// ----------------------------------------------------------------- framing

enum FrameError {
    TooLarge,
    Io(io::Error),
    Eof,
}

fn write_frame(c: &mut impl Write, body: &[u8]) -> io::Result<()> {
    let len = u32::try_from(body.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame too large"))?;
    let mut buf = Vec::with_capacity(4 + body.len());
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(body);
    c.write_all(&buf)?;
    c.flush()
}

/// Reads one frame of at most `max` bytes. `Eof` when the peer closed
/// before sending anything (a probe connection).
fn read_frame(c: &mut impl Read, max: usize) -> Result<Vec<u8>, FrameError> {
    let mut head = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        match c.read(&mut head[got..]) {
            Ok(0) if got == 0 => return Err(FrameError::Eof),
            Ok(0) => return Err(FrameError::Io(io::ErrorKind::UnexpectedEof.into())),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(FrameError::Io(e)),
        }
    }
    let len = u32::from_be_bytes(head) as usize;
    if len > max {
        return Err(FrameError::TooLarge);
    }
    let mut body = vec![0u8; len];
    c.read_exact(&mut body).map_err(FrameError::Io)?;
    Ok(body)
}

// ---------------------------------------------------------------- requests

/// One call: a function name and its string arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub func: String,
    pub args: Vec<String>,
    /// The caller's working directory (resolve relative paths against it).
    pub cwd: Option<String>,
}

impl Request {
    pub fn new(func: &str, args: &[&str]) -> Request {
        Request {
            func: func.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
            cwd: std::env::current_dir().ok().map(|p| p.to_string_lossy().into_owned()),
        }
    }

    /// The first argument, or "".
    pub fn arg(&self) -> &str {
        self.args.first().map_or("", String::as_str)
    }

    fn encode(&self) -> Vec<u8> {
        json!({ "v": VERSION, "fn": self.func, "args": self.args, "cwd": self.cwd }).to_string().into_bytes()
    }

    fn decode(body: &[u8]) -> Result<Request, String> {
        let v: Value = serde_json::from_slice(body).map_err(|e| format!("bad request: {e}"))?;
        if v.get("v").and_then(Value::as_u64) != Some(VERSION) {
            return Err("unsupported protocol version".into());
        }
        let func = v.get("fn").and_then(Value::as_str).ok_or("bad request: no function name")?.to_string();
        let args = match v.get("args") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(a)) => a.iter().map(|x| x.as_str().map(str::to_string).ok_or("bad request: arguments must be strings")).collect::<Result<_, _>>()?,
            Some(_) => return Err("bad request: args must be a list".into()),
        };
        let cwd = v.get("cwd").and_then(Value::as_str).map(str::to_string);
        Ok(Request { func, args, cwd })
    }
}

fn encode_reply(r: &Result<String, String>) -> Vec<u8> {
    match r {
        Ok(v) => json!({ "ok": v }),
        Err(e) => json!({ "err": e }),
    }
    .to_string()
    .into_bytes()
}

fn decode_reply(body: &[u8]) -> Result<String, Error> {
    let v: Value = serde_json::from_slice(body).map_err(|e| Error::Protocol(e.to_string()))?;
    if let Some(ok) = v.get("ok").and_then(Value::as_str) {
        return Ok(ok.to_string());
    }
    match v.get("err").and_then(Value::as_str) {
        Some(e) => Err(Error::Remote(e.to_string())),
        None => Err(Error::Protocol("reply without ok/err".into())),
    }
}

fn valid_app_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 96 && !id.starts_with('.') && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

// ----------------------------------------------------------------- options

/// Settings for [`Options::acquire`] and [`Options::client`]; the defaults
/// suit apps. `Options::new("dev.you.app").acquire()` is [`acquire`].
#[derive(Clone)]
pub struct Options {
    app_id: String,
    runtime_dir: Option<PathBuf>,
    handler_timeout: Duration,
    io_timeout: Duration,
    wait: Duration,
    max_request: usize,
    max_reply: usize,
    max_connections: usize,
    #[cfg(test)]
    hop: Option<mpsc::Sender<Job>>,
}

impl Options {
    pub fn new(app_id: &str) -> Options {
        Options {
            app_id: app_id.to_string(),
            runtime_dir: None,
            handler_timeout: Duration::from_secs(5),
            io_timeout: Duration::from_secs(2),
            wait: Duration::from_secs(2),
            max_request: MAX_REQUEST,
            max_reply: MAX_REPLY,
            max_connections: 16,
            #[cfg(test)]
            hop: None,
        }
    }

    /// Where the endpoint lives (default [`dirs::runtime`](crate::platform::dirs::runtime)).
    /// Tests use a private temporary directory.
    pub fn runtime_dir(mut self, dir: impl Into<PathBuf>) -> Options {
        self.runtime_dir = Some(dir.into());
        self
    }

    /// How long the server waits for a UI-thread handler (default 5 s);
    /// the caller then gets an error. Clients wait this plus 1 s.
    pub fn handler_timeout(mut self, d: Duration) -> Options {
        self.handler_timeout = d;
        self
    }

    /// Deadline of each read and write on a connection (default 2 s).
    pub fn io_timeout(mut self, d: Duration) -> Options {
        self.io_timeout = d;
        self
    }

    /// How long a launch waits for a primary that is still starting
    /// (holds the lock but does not listen yet; default 2 s).
    pub fn wait(mut self, d: Duration) -> Options {
        self.wait = d;
        self
    }

    /// Size limits in bytes (defaults [`MAX_REQUEST`], [`MAX_REPLY`]).
    pub fn max_sizes(mut self, request: usize, reply: usize) -> Options {
        self.max_request = request;
        self.max_reply = reply;
        self
    }

    /// Requests served at once (default 16); more get a "busy" error.
    pub fn max_connections(mut self, n: usize) -> Options {
        self.max_connections = n.max(1);
        self
    }

    #[cfg(test)]
    fn hop(mut self, tx: mpsc::Sender<Job>) -> Options {
        self.hop = Some(tx);
        self
    }

    fn runtime(&self) -> PathBuf {
        self.runtime_dir.clone().unwrap_or_else(crate::platform::dirs::runtime)
    }

    /// Becomes the primary instance or finds the running one.
    pub fn acquire(self) -> Instance {
        if !valid_app_id(&self.app_id) {
            return Instance::Primary(Server::unavailable(self, "invalid app id (use letters, digits, '.', '_', '-')".into()));
        }
        let ep = match ipc::endpoint(&self.app_id, &self.runtime(), true) {
            Ok(ep) => ep,
            Err(e) => return Instance::Primary(Server::unavailable(self, e.to_string())),
        };
        let deadline = Instant::now() + self.wait;
        loop {
            match ipc::claim(&ep) {
                Ok(Claim::Primary(l)) => return Instance::Primary(Server::start(self, l, ep.describe())),
                Ok(Claim::Taken) => {}
                Err(e) => return Instance::Primary(Server::unavailable(self, e.to_string())),
            }
            // Someone holds it: a live primary answers (this probe is an
            // empty connection, which the server ignores).
            match ipc::connect(&ep, self.io_timeout) {
                Ok(_) => return Instance::Secondary(self.client()),
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                    return Instance::Primary(Server::unavailable(self, e.to_string()))
                }
                Err(_) => {}
            }
            if Instant::now() >= deadline {
                let why = format!("another instance holds {} but does not answer", ep.describe());
                return Instance::Primary(Server::unavailable(self, why));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// A client for these options (no connection is made yet).
    pub fn client(&self) -> Client {
        Client {
            app_id: self.app_id.clone(),
            runtime: self.runtime(),
            timeout: self.handler_timeout + Duration::from_secs(1),
            max_reply: self.max_reply,
        }
    }
}

// ------------------------------------------------------------------ client

/// Calls functions of the running instance. Cheap; connects per call.
#[derive(Debug, Clone)]
pub struct Client {
    app_id: String,
    runtime: PathBuf,
    timeout: Duration,
    max_reply: usize,
}

impl Client {
    /// A client of `app_id` with the default options.
    pub fn new(app_id: &str) -> Client {
        Options::new(app_id).client()
    }

    /// Total time a call may take (default 6 s).
    pub fn timeout(mut self, d: Duration) -> Client {
        self.timeout = d;
        self
    }

    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    /// `func(arg)` on the running instance.
    pub fn call(&self, func: &str, arg: &str) -> Result<String, Error> {
        self.request(&Request::new(func, &[arg]))
    }

    /// `func(args..)` on the running instance.
    pub fn call_args(&self, func: &str, args: &[&str]) -> Result<String, Error> {
        self.request(&Request::new(func, args))
    }

    /// Forwards a launch's arguments to the running instance (`activate`).
    pub fn activate<S: AsRef<str>>(&self, args: impl IntoIterator<Item = S>) -> Result<String, Error> {
        let owned: Vec<String> = args.into_iter().map(|a| a.as_ref().to_string()).collect();
        let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
        self.call_args("activate", &refs)
    }

    /// True when an instance answers `ping`.
    pub fn ping(&self) -> bool {
        self.call("ping", "").is_ok()
    }

    /// Sends any request.
    pub fn request(&self, req: &Request) -> Result<String, Error> {
        if !valid_app_id(&self.app_id) {
            return Err(Error::Unavailable("invalid app id".into()));
        }
        let ep = ipc::endpoint(&self.app_id, &self.runtime, false).map_err(io_error)?;
        let start = Instant::now();
        let mut c = ipc::connect(&ep, self.timeout).map_err(io_error)?;
        let left = self.timeout.saturating_sub(start.elapsed()).max(Duration::from_millis(1));
        c.set_timeout(Some(left)).map_err(Error::Io)?;
        write_frame(&mut c, &req.encode()).map_err(io_error)?;
        match read_frame(&mut c, self.max_reply) {
            Ok(body) => decode_reply(&body),
            Err(FrameError::TooLarge) => Err(Error::TooLarge),
            Err(FrameError::Eof) => Err(Error::Protocol("connection closed without a reply".into())),
            Err(FrameError::Io(e)) => Err(io_error(e)),
        }
    }
}

/// `func(arg)` on the running instance of `app_id`: what a CLI such as
/// `myapp call FN ARG` does.
pub fn call(app_id: &str, func: &str, arg: &str) -> Result<String, Error> {
    Client::new(app_id).call(func, arg)
}

/// `func(args..)` on the running instance of `app_id`.
pub fn call_args(app_id: &str, func: &str, args: &[&str]) -> Result<String, Error> {
    Client::new(app_id).call_args(func, args)
}

// ------------------------------------------------------------------ server

/// What [`acquire`] found.
pub enum Instance {
    /// This process is the app: register handlers on the server.
    Primary(Server),
    /// Another process is: talk to it through the client.
    Secondary(Client),
}

/// Becomes the primary instance of `app_id`, or finds the running one
/// (waiting up to 2 s for one that is starting). Never fails: without IPC
/// the result is a primary that cannot be reached ([`Server::unavailable`]).
pub fn acquire(app_id: &str) -> Instance {
    Options::new(app_id).acquire()
}

/// The default single-instance launch: the primary gets its server; a
/// second launch forwards its command-line arguments to the primary
/// (`activate`, which raises the managed window) and exits with status 0
/// (1, with a message, when the primary does not answer).
pub fn acquire_or_forward(app_id: &str) -> Server {
    match acquire(app_id) {
        Instance::Primary(s) => s,
        Instance::Secondary(c) => {
            let args: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
            match c.activate(&args) {
                Ok(_) => std::process::exit(0),
                Err(e) => {
                    eprintln!("{app_id}: the running instance did not answer: {e}");
                    std::process::exit(1)
                }
            }
        }
    }
}

type Job = Box<dyn FnOnce() + Send>;
type Handler = Rc<dyn Fn(&Request) -> Result<String, String>>;
type ActivateFn = Rc<dyn Fn(&Request)>;

/// How requests reach the UI thread: the app loop ([`crate::task::post`]),
/// or (unit tests) a channel the test thread drains.
enum Hop {
    Ui,
    #[cfg(test)]
    Channel(std::sync::Mutex<mpsc::Sender<Job>>),
}

impl Hop {
    fn send(&self, job: Job) {
        match self {
            Hop::Ui => crate::task::post(job),
            #[cfg(test)]
            Hop::Channel(tx) => {
                let _ = tx.lock().unwrap().send(job);
            }
        }
    }
}

struct Shared {
    id: u64,
    app_id: Arc<str>,
    listener: ipc::Listener,
    stop: AtomicBool,
    active: AtomicUsize,
    hop: Hop,
    opts: Options,
}

#[derive(Default)]
struct Registry {
    fns: RefCell<HashMap<String, Handler>>,
    window: RefCell<Option<crate::AppWindow>>,
    on_activate: RefCell<Option<ActivateFn>>,
}

thread_local! {
    static REGISTRY: RefCell<HashMap<u64, Rc<Registry>>> = RefCell::new(HashMap::new());
}
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn registry(id: u64) -> Rc<Registry> {
    REGISTRY.with(|r| r.borrow_mut().entry(id).or_default().clone())
}

/// The primary instance: receives calls while it lives (drop it to stop
/// serving; the endpoint is released). Lives on the UI thread.
pub struct Server {
    id: u64,
    app_id: Arc<str>,
    shared: Option<Arc<Shared>>,
    thread: Option<std::thread::JoinHandle<()>>,
    unavailable: Option<String>,
    endpoint: String,
    _ui_thread: PhantomData<Rc<()>>,
}

impl Server {
    fn unavailable(opts: Options, why: String) -> Server {
        eprintln!("fltk_omarchy: single-instance IPC unavailable for {} ({why}); running as the primary instance without IPC", opts.app_id);
        Server {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            app_id: opts.app_id.as_str().into(),
            shared: None,
            thread: None,
            unavailable: Some(why),
            endpoint: String::new(),
            _ui_thread: PhantomData,
        }
    }

    fn start(opts: Options, listener: ipc::Listener, endpoint: String) -> Server {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        #[cfg(test)]
        let hop = match &opts.hop {
            Some(tx) => Hop::Channel(std::sync::Mutex::new(tx.clone())),
            None => Hop::Ui,
        };
        #[cfg(not(test))]
        let hop = Hop::Ui;
        let app_id: Arc<str> = opts.app_id.as_str().into();
        let shared = Arc::new(Shared { id, app_id: app_id.clone(), listener, stop: AtomicBool::new(false), active: AtomicUsize::new(0), hop, opts });
        let s2 = shared.clone();
        let thread = std::thread::Builder::new().name("omarchy-instance".into()).spawn(move || accept_loop(s2));
        match thread {
            Ok(t) => Server { id, app_id, shared: Some(shared), thread: Some(t), unavailable: None, endpoint, _ui_thread: PhantomData },
            Err(e) => {
                let opts = shared.opts.clone();
                drop(shared);
                Server::unavailable(opts, e.to_string())
            }
        }
    }

    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    /// Why this primary cannot be reached, when IPC is unavailable.
    pub fn unavailable_reason(&self) -> Option<&str> {
        self.unavailable.as_deref()
    }

    /// True when other processes can reach this instance.
    pub fn is_listening(&self) -> bool {
        self.shared.is_some()
    }

    /// The socket path or pipe name ("" without IPC).
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Registers `name`: `f` gets the first argument ("" when none) and
    /// returns the reply. Runs on the UI thread.
    pub fn handle(&self, name: &str, f: impl Fn(&str) -> String + 'static) -> &Server {
        self.handle_request(name, move |r| Ok(f(r.arg())))
    }

    /// Registers `name` with the whole request (every argument, the
    /// caller's directory); `Err` is sent back as an error.
    pub fn handle_request(&self, name: &str, f: impl Fn(&Request) -> Result<String, String> + 'static) -> &Server {
        registry(self.id).fns.borrow_mut().insert(name.to_string(), Rc::new(f));
        self
    }

    /// Called for `activate` (a second launch, with its arguments) before
    /// the managed window is raised.
    pub fn on_activate(&self, f: impl Fn(&Request) + 'static) -> &Server {
        *registry(self.id).on_activate.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// Lets IPC show, hide, toggle and raise `win` (built-ins `show`,
    /// `raise`, `hide`, `toggle`, `window`, `is_active`; `activate` raises
    /// it too).
    pub fn manage(&self, win: &crate::AppWindow) -> &Server {
        *registry(self.id).window.borrow_mut() = Some(win.clone());
        self
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(sh) = self.shared.take() {
            sh.stop.store(true, Ordering::SeqCst);
            sh.listener.wake();
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
        let _ = REGISTRY.try_with(|r| r.borrow_mut().remove(&self.id));
    }
}

fn accept_loop(sh: Arc<Shared>) {
    let mut errors = 0u32;
    loop {
        let conn = sh.listener.accept();
        if sh.stop.load(Ordering::SeqCst) {
            return;
        }
        let mut conn = match conn {
            Ok(c) => {
                errors = 0;
                c
            }
            Err(_) => {
                errors += 1;
                std::thread::sleep(Duration::from_millis(if errors > 10 { 500 } else { 20 }));
                continue;
            }
        };
        if sh.active.fetch_add(1, Ordering::SeqCst) >= sh.opts.max_connections {
            sh.active.fetch_sub(1, Ordering::SeqCst);
            refuse(&mut conn, "busy: too many requests at once");
            continue;
        }
        let s2 = sh.clone();
        let spawned = std::thread::Builder::new().name("omarchy-instance-conn".into()).spawn(move || {
            serve(&s2, conn);
            s2.active.fetch_sub(1, Ordering::SeqCst);
        });
        if spawned.is_err() {
            sh.active.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

fn refuse(conn: &mut Conn, why: &str) {
    let _ = conn.set_timeout(Some(Duration::from_millis(200)));
    let _ = write_frame(conn, &encode_reply(&Err(why.to_string())));
}

fn serve(sh: &Shared, mut conn: Conn) {
    let o = &sh.opts;
    if conn.set_timeout(Some(o.io_timeout)).is_err() {
        return;
    }
    let body = match read_frame(&mut conn, o.max_request) {
        Ok(b) => b,
        Err(FrameError::TooLarge) => return refuse(&mut conn, &format!("request too large (limit {} bytes)", o.max_request)),
        Err(FrameError::Eof) | Err(FrameError::Io(_)) => return,
    };
    let req = match Request::decode(&body) {
        Ok(r) => r,
        Err(e) => return refuse(&mut conn, &e),
    };
    let (tx, rx) = mpsc::sync_channel::<Result<String, String>>(1);
    let (id, app_id) = (sh.id, sh.app_id.clone());
    sh.hop.send(Box::new(move || {
        let _ = tx.send(dispatch(id, &app_id, &req));
    }));
    let mut reply = match rx.recv_timeout(o.handler_timeout) {
        Ok(r) => r,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(format!("timeout: the app did not answer within {} ms", o.handler_timeout.as_millis())),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err("the handler failed".to_string()),
    };
    let mut bytes = encode_reply(&reply);
    if bytes.len() > o.max_reply {
        reply = Err(format!("reply too large (limit {} bytes)", o.max_reply));
        bytes = encode_reply(&reply);
    }
    let _ = write_frame(&mut conn, &bytes);
}

fn window_json(w: &crate::AppWindow) -> String {
    use fltk::prelude::WindowExt;
    json!({ "shown": w.shown(), "active": w.is_active() }).to_string()
}

/// Runs on the UI thread.
fn dispatch(id: u64, app_id: &str, req: &Request) -> Result<String, String> {
    use fltk::prelude::{WidgetExt, WindowExt};
    let reg = REGISTRY.with(|r| r.borrow().get(&id).cloned()).unwrap_or_default();
    let f = reg.fns.borrow().get(&req.func).cloned();
    if let Some(f) = f {
        return f(req);
    }
    let win = reg.window.borrow().clone();
    let shown = |w: &crate::AppWindow| json!({ "shown": w.shown() }).to_string();
    match (req.func.as_str(), win) {
        ("ping", _) => Ok(json!({ "pid": std::process::id(), "app": app_id }).to_string()),
        ("functions", w) => {
            let mut names: Vec<String> = reg.fns.borrow().keys().cloned().collect();
            names.extend(["ping", "functions", "activate", "quit"].map(String::from));
            if w.is_some() {
                names.extend(["show", "raise", "hide", "toggle", "window", "is_active"].map(String::from));
            }
            names.sort();
            names.dedup();
            Ok(json!(names).to_string())
        }
        ("activate", w) => {
            let cb = reg.on_activate.borrow().clone();
            if let Some(cb) = cb {
                cb(req);
            }
            if let Some(mut w) = w {
                w.raise();
            }
            Ok("true".into())
        }
        ("quit", _) => {
            crate::App::quit();
            Ok("true".into())
        }
        ("show" | "raise", Some(mut w)) => {
            w.raise();
            Ok(shown(&w))
        }
        ("hide", Some(mut w)) => {
            w.hide();
            Ok(shown(&w))
        }
        ("toggle", Some(mut w)) => {
            w.toggle();
            Ok(shown(&w))
        }
        ("window", Some(w)) => Ok(window_json(&w)),
        ("is_active", Some(w)) => Ok(w.is_active().to_string()),
        ("show" | "raise" | "hide" | "toggle" | "window" | "is_active", None) => Err(format!("{}: the app manages no window", req.func)),
        (name, _) => Err(format!("unknown function {name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    /// A private runtime dir for one test, under the system temp dir (a
    /// short path: Unix socket paths are limited to ~100 bytes). Drop
    /// removes exactly the files the instance layer creates and then the
    /// empty directories; never a whole tree.
    struct TempRuntime(PathBuf);

    impl TempRuntime {
        fn new() -> TempRuntime {
            static N: AtomicU32 = AtomicU32::new(0);
            // A short root: socket paths must fit sun_path, and some systems
            // have long per-user temp dirs.
            let tmp = std::env::temp_dir();
            let root = if cfg!(unix) && tmp.as_os_str().len() > 24 && std::path::Path::new("/tmp").is_dir() { PathBuf::from("/tmp") } else { tmp };
            let p = root.join(format!("fo-ipc-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir(&p).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            TempRuntime(p)
        }
        fn opts(&self, app: &str) -> Options {
            Options::new(app).runtime_dir(&self.0)
        }
    }

    impl Drop for TempRuntime {
        fn drop(&mut self) {
            for app in ["dev.test.a", "dev.test.b"] {
                let d = self.0.join(app);
                // Only a real directory (never through a symlink).
                if !std::fs::symlink_metadata(&d).is_ok_and(|m| m.file_type().is_dir()) {
                    continue;
                }
                let _ = std::fs::remove_file(d.join("instance.sock"));
                let _ = std::fs::remove_file(d.join("instance.lock"));
                let _ = std::fs::remove_dir(&d);
            }
            let _ = std::fs::remove_dir(&self.0);
        }
    }

    /// Runs posted jobs on this thread (the test's "UI thread") until
    /// `done` is true or 10 s pass.
    fn pump(rx: &mpsc::Receiver<Job>, done: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(10);
        while !done() && Instant::now() < end {
            if let Ok(job) = rx.recv_timeout(Duration::from_millis(5)) {
                job();
            }
        }
    }

    fn primary(opts: Options) -> Server {
        match opts.acquire() {
            Instance::Primary(s) => s,
            Instance::Secondary(_) => panic!("expected a primary"),
        }
    }

    /// Calls on a thread while this thread pumps; returns the result.
    fn call_pumped(rx: &mpsc::Receiver<Job>, client: Client, req: Request) -> Result<String, Error> {
        let t = std::thread::spawn(move || client.request(&req));
        pump(rx, || t.is_finished());
        t.join().unwrap()
    }

    #[test]
    fn frames_and_messages_round_trip() {
        let req = Request { func: "drive".into(), args: vec!["thread".into(), "a/b \"c\" ü".into()], cwd: Some("/x".into()) };
        let mut buf = Vec::new();
        write_frame(&mut buf, &req.encode()).unwrap();
        assert_eq!(u32::from_be_bytes(buf[..4].try_into().unwrap()) as usize, buf.len() - 4);
        let body = read_frame(&mut &buf[..], 1 << 10).ok().unwrap();
        assert_eq!(Request::decode(&body).unwrap(), req);
        assert_eq!(req.arg(), "thread");
        assert!(matches!(read_frame(&mut &buf[..], 8), Err(FrameError::TooLarge)));
        assert!(matches!(read_frame(&mut &b""[..], 8), Err(FrameError::Eof)));
        assert!(matches!(read_frame(&mut &b"\0\0"[..], 8), Err(FrameError::Io(_))));
        assert_eq!(decode_reply(&encode_reply(&Ok("{\"a\":1}".into()))).unwrap(), "{\"a\":1}");
        assert!(matches!(decode_reply(&encode_reply(&Err("nope".into()))), Err(Error::Remote(e)) if e == "nope"));
        assert!(Request::decode(br#"{"v":2,"fn":"x"}"#).is_err());
        assert!(Request::decode(br#"{"v":1,"fn":"x","args":[1]}"#).is_err());
        assert_eq!(Request::decode(br#"{"v":1,"fn":"x"}"#).unwrap().args, Vec::<String>::new());
        assert!(valid_app_id("io.github.avillagran.omarchy-t3-code"));
        assert!(!valid_app_id("../x") && !valid_app_id("a/b") && !valid_app_id("") && !valid_app_id(".hidden"));
    }

    #[test]
    fn second_acquire_is_a_secondary_and_calls_reach_handlers() {
        let rt = TempRuntime::new();
        let (tx, rx) = mpsc::channel();
        let server = primary(rt.opts("dev.test.a").hop(tx));
        assert!(server.is_listening(), "{:?}", server.unavailable_reason());
        server.handle("echo", |a| format!("echo:{a}"));
        server.handle_request("join", |r| if r.args.is_empty() { Err("no args".into()) } else { Ok(r.args.join("+")) });
        let client = match rt.opts("dev.test.a").wait(Duration::from_millis(300)).acquire() {
            Instance::Secondary(c) => c,
            Instance::Primary(_) => panic!("expected a secondary"),
        };
        assert_eq!(call_pumped(&rx, client.clone(), Request::new("echo", &["hi"])).unwrap(), "echo:hi");
        assert_eq!(call_pumped(&rx, client.clone(), Request::new("join", &["a", "b", "c"])).unwrap(), "a+b+c");
        assert!(matches!(call_pumped(&rx, client.clone(), Request::new("join", &[])), Err(Error::Remote(e)) if e == "no args"));
        assert!(matches!(call_pumped(&rx, client.clone(), Request::new("nope", &[])), Err(Error::Remote(e)) if e.contains("unknown function")));
        let ping = call_pumped(&rx, client.clone(), Request::new("ping", &[])).unwrap();
        assert!(ping.contains(&std::process::id().to_string()) && ping.contains("dev.test.a"), "{ping}");
        let fns = call_pumped(&rx, client.clone(), Request::new("functions", &[])).unwrap();
        assert!(fns.contains("\"echo\"") && fns.contains("\"ping\"") && !fns.contains("toggle"), "{fns}");
        assert!(matches!(call_pumped(&rx, client.clone(), Request::new("toggle", &[])), Err(Error::Remote(e)) if e.contains("no window")));
        // A request carries the caller's directory.
        server.handle_request("cwd", |r| Ok(r.cwd.clone().unwrap_or_default()));
        let cwd = call_pumped(&rx, client.clone(), Request::new("cwd", &[])).unwrap();
        assert_eq!(cwd, std::env::current_dir().unwrap().to_string_lossy());
        // Another app id is its own instance.
        assert!(primary(rt.opts("dev.test.b")).is_listening());
        // After the primary stops, the next launch is the primary.
        drop(server);
        assert!(matches!(client.call("echo", "x"), Err(Error::NotRunning)));
        assert!(primary(rt.opts("dev.test.a")).is_listening());
    }

    #[cfg(unix)]
    #[test]
    fn stale_socket_is_taken_over_and_files_are_private() {
        use std::os::unix::fs::{FileTypeExt, PermissionsExt};
        let rt = TempRuntime::new();
        let dir = rt.0.join("dev.test.a");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        // A crashed primary: its socket file stays, nobody listens, the lock is free.
        let sock = dir.join("instance.sock");
        drop(std::os::unix::net::UnixListener::bind(&sock).unwrap());
        assert!(std::fs::symlink_metadata(&sock).unwrap().file_type().is_socket());
        assert!(matches!(rt.opts("dev.test.a").client().call("ping", ""), Err(Error::NotRunning)));
        let (tx, rx) = mpsc::channel();
        let server = primary(rt.opts("dev.test.a").hop(tx));
        assert!(server.is_listening(), "{:?}", server.unavailable_reason());
        assert_eq!(server.endpoint(), sock.to_string_lossy());
        let mode = |p: &std::path::Path| std::fs::symlink_metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&sock), 0o600);
        assert_eq!(mode(&dir.join("instance.lock")), 0o600);
        assert!(call_pumped(&rx, rt.opts("dev.test.a").client(), Request::new("ping", &[])).is_ok());
        drop(server);
        // The primary removes its own socket on exit (its server thread does
        // it right after the drop).
        let gone = (0..100).any(|_| {
            if std::fs::symlink_metadata(&sock).is_err() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            false
        });
        assert!(gone, "socket left behind");
    }

    #[cfg(unix)]
    #[test]
    fn long_runtime_dirs_use_a_short_socket_path() {
        use std::os::unix::fs::PermissionsExt;
        let rt = TempRuntime::new();
        let mut deep = rt.0.clone();
        for k in 0..6 {
            deep = deep.join(format!("a-rather-long-directory-name-{k}"));
            std::fs::create_dir(&deep).unwrap();
            std::fs::set_permissions(&deep, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let (tx, rx) = mpsc::channel();
        let server = primary(Options::new("dev.test.long").runtime_dir(&deep).hop(tx));
        assert!(server.is_listening(), "{:?}", server.unavailable_reason());
        let ep = server.endpoint().to_string();
        assert!(ep.len() <= 100 && ep.starts_with("/tmp/fltk-omarchy-"), "{ep}");
        let client = Options::new("dev.test.long").runtime_dir(&deep).client();
        assert!(call_pumped(&rx, client, Request::new("ping", &[])).is_ok());
        drop(server);
        // Remove what the test made in /tmp, file by file (never recursively).
        let dir = std::path::Path::new(&ep).parent().unwrap().to_path_buf();
        let _ = std::fs::remove_file(dir.join("instance.lock"));
        let _ = std::fs::remove_dir(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn unsafe_runtime_dirs_are_refused() {
        use std::os::unix::fs::PermissionsExt;
        let rt = TempRuntime::new();
        // A shared runtime dir (like /tmp) gets a private per-user subdir.
        let shared = rt.0.join("shared");
        std::fs::create_dir(&shared).unwrap();
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o777)).unwrap();
        let s = primary(Options::new("dev.test.a").runtime_dir(&shared));
        assert!(s.is_listening(), "{:?}", s.unavailable_reason());
        let private = shared.join(format!("fltk-omarchy-{}", unsafe { libc::geteuid() }));
        assert!(s.endpoint().starts_with(&*private.to_string_lossy()));
        drop(s);
        // A symlink where the app dir should be is not followed.
        let target = rt.0.join("elsewhere");
        std::fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, rt.0.join("dev.test.b")).unwrap();
        let s = primary(rt.opts("dev.test.b"));
        assert!(!s.is_listening() && s.unavailable_reason().is_some());
        assert!(std::fs::read_dir(&target).unwrap().next().is_none(), "nothing was created through the link");
        // Cleanup of this test's own entries (files and empty dirs only).
        let a = private.join("dev.test.a");
        let _ = std::fs::remove_file(a.join("instance.lock"));
        let _ = std::fs::remove_dir(&a);
        let _ = std::fs::remove_dir(&private);
        let _ = std::fs::remove_dir(&shared);
        let _ = std::fs::remove_file(rt.0.join("dev.test.b"));
        let _ = std::fs::remove_dir(&target);
    }

    #[test]
    fn concurrent_clients_all_get_their_reply() {
        let rt = TempRuntime::new();
        let (tx, rx) = mpsc::channel();
        let server = primary(rt.opts("dev.test.a").hop(tx).max_connections(64));
        server.handle_request("sq", |r| Ok((r.arg().parse::<u64>().unwrap() * r.arg().parse::<u64>().unwrap()).to_string()));
        let threads: Vec<_> = (0..8u64)
            .map(|t| {
                let c = rt.opts("dev.test.a").client();
                std::thread::spawn(move || (0..10u64).all(|i| {
                    let n = t * 100 + i;
                    c.call("sq", &n.to_string()).ok() == Some((n * n).to_string())
                }))
            })
            .collect();
        pump(&rx, || threads.iter().all(|t| t.is_finished()));
        assert!(threads.into_iter().all(|t| t.join().unwrap()));
    }

    #[test]
    fn size_limits_are_enforced() {
        let rt = TempRuntime::new();
        let (tx, rx) = mpsc::channel();
        let server = primary(rt.opts("dev.test.a").hop(tx).max_sizes(256, 512));
        server.handle("big", |n| "x".repeat(n.parse().unwrap()));
        let c = rt.opts("dev.test.a").client();
        let long = "y".repeat(300);
        assert!(matches!(call_pumped(&rx, c.clone(), Request::new("big", &[&long])), Err(Error::Remote(e)) if e.contains("request too large")));
        assert_eq!(call_pumped(&rx, c.clone(), Request::new("big", &["100"])).unwrap().len(), 100);
        assert!(matches!(call_pumped(&rx, c.clone(), Request::new("big", &["600"])), Err(Error::Remote(e)) if e.contains("reply too large")));
        // The client's own limit.
        let small = Client { max_reply: 16, ..c };
        assert!(matches!(call_pumped(&rx, small, Request::new("big", &["100"])), Err(Error::TooLarge)));
    }

    #[test]
    fn timeouts_bound_handlers_clients_and_idle_connections() {
        let rt = TempRuntime::new();
        let (tx, rx) = mpsc::channel();
        let server = primary(rt.opts("dev.test.a").hop(tx).handler_timeout(Duration::from_millis(200)).io_timeout(Duration::from_millis(200)));
        server.handle("slow", |_| {
            std::thread::sleep(Duration::from_millis(400));
            "late".into()
        });
        server.handle("ok", |_| "ok".into());
        let c = rt.opts("dev.test.a").client();
        // The UI thread is busy: the server answers with its own timeout.
        let t0 = Instant::now();
        let r = call_pumped(&rx, c.clone().timeout(Duration::from_secs(3)), Request::new("slow", &[]));
        assert!(matches!(&r, Err(Error::Remote(e)) if e.starts_with("timeout")), "{r:?}");
        assert!(t0.elapsed() < Duration::from_secs(2));
        // Nobody drains the UI queue and the client gives up first.
        let t0 = Instant::now();
        let r = c.clone().timeout(Duration::from_millis(100)).call("ok", "");
        assert!(matches!(r, Err(Error::Timeout)), "{r:?}");
        assert!(t0.elapsed() < Duration::from_secs(1));
        while rx.try_recv().is_ok() {} // drop the stale jobs
        // A client that connects and never sends is dropped after io_timeout.
        let ep = ipc::endpoint("dev.test.a", &rt.0, false).unwrap();
        let mut idle = ipc::connect(&ep, Duration::from_secs(3)).unwrap();
        idle.write_all(&[0, 0]).unwrap(); // half a header
        let t0 = Instant::now();
        let mut buf = [0u8; 8];
        let n = idle.read(&mut buf).unwrap_or(0);
        assert_eq!(n, 0, "closed without a reply");
        assert!(t0.elapsed() < Duration::from_secs(2), "{:?}", t0.elapsed());
        // The server still serves.
        assert_eq!(call_pumped(&rx, c, Request::new("ok", &[])).unwrap(), "ok");
    }

    #[test]
    fn too_many_connections_get_busy() {
        let rt = TempRuntime::new();
        let (tx, rx) = mpsc::channel();
        let _server = primary(rt.opts("dev.test.a").hop(tx).max_connections(1).io_timeout(Duration::from_millis(500)));
        let ep = ipc::endpoint("dev.test.a", &rt.0, false).unwrap();
        // One connection occupies the only slot (sends nothing yet).
        let _hold = ipc::connect(&ep, Duration::from_secs(2)).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        let r = rt.opts("dev.test.a").client().call("ping", "");
        assert!(matches!(&r, Err(Error::Remote(e)) if e.starts_with("busy")), "{r:?}");
        drop(rx);
    }

    #[test]
    fn invalid_app_ids_run_without_ipc() {
        let s = primary(Options::new("../evil"));
        assert!(!s.is_listening());
        assert!(s.unavailable_reason().unwrap().contains("invalid app id"));
        assert!(matches!(call("../evil", "ping", ""), Err(Error::Unavailable(_))));
    }
}
