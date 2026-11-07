//! xdg-activation-v1 on the existing FLTK Wayland connection. A private
//! event queue prevents handshakes from reentering application callbacks.
//! Protocol descriptors: see xdg-activation-LICENSE in this directory.
use super::*;
use std::{
    cell::{Cell, RefCell},
    ffi::{c_char, c_int, c_void, CStr, CString},
    ptr,
    task::Poll,
    time::{Duration, Instant},
};

#[repr(C)]
struct Message {
    name: *const c_char,
    signature: *const c_char,
    types: *const *const Interface,
}
#[repr(C)]
struct Interface {
    name: *const c_char,
    version: c_int,
    method_count: c_int,
    methods: *const Message,
    event_count: c_int,
    events: *const Message,
}
// SAFETY: these protocol descriptors point only to static immutable data.
unsafe impl Sync for Message {}
unsafe impl Sync for Interface {}
struct Types<const N: usize>([*const Interface; N]);
// SAFETY: immutable pointers to the core/generated static interfaces.
unsafe impl<const N: usize> Sync for Types<N> {}
#[link(name = "wayland-client")]
extern "C" {
    static wl_registry_interface: Interface;
    static wl_surface_interface: Interface;
    static wl_seat_interface: Interface;
    static wl_callback_interface: Interface;
    fn wl_proxy_marshal_flags(
        proxy: *mut c_void,
        opcode: u32,
        interface: *const c_void,
        version: u32,
        flags: u32,
        ...
    ) -> *mut c_void;
    fn wl_proxy_get_version(proxy: *mut c_void) -> u32;
    fn wl_proxy_add_listener(
        proxy: *mut c_void,
        implementation: *mut *const c_void,
        data: *mut c_void,
    ) -> c_int;
    fn wl_proxy_destroy(proxy: *mut c_void);
    fn wl_proxy_create_wrapper(proxy: *mut c_void) -> *mut c_void;
    fn wl_proxy_wrapper_destroy(proxy: *mut c_void);
    fn wl_proxy_set_queue(proxy: *mut c_void, queue: *mut c_void);
    fn wl_display_create_queue(display: *mut c_void) -> *mut c_void;
    fn wl_event_queue_destroy(queue: *mut c_void);
    fn wl_display_prepare_read_queue(display: *mut c_void, queue: *mut c_void) -> c_int;
    fn wl_display_cancel_read(display: *mut c_void);
    fn wl_display_read_events(display: *mut c_void) -> c_int;
    fn wl_display_dispatch_queue_pending(display: *mut c_void, queue: *mut c_void) -> c_int;
    fn wl_display_flush(display: *mut c_void) -> c_int;
    fn wl_display_get_error(display: *mut c_void) -> c_int;
    fn wl_display_get_fd(display: *mut c_void) -> c_int;
    #[link_name = "_Z13fl_wl_surfaceP10wld_window"]
    fn fl_wl_surface(window: *mut c_void) -> *mut c_void;
    #[link_name = "_ZN2Fl13screen_driverEv"]
    fn fl_screen_driver() -> *mut c_void;
    #[link_name = "_ZN24Fl_Wayland_Screen_Driver10get_serialEv"]
    fn get_serial(driver: *mut c_void) -> u32;
    #[link_name = "_ZN24Fl_Wayland_Screen_Driver11get_wl_seatEv"]
    fn get_seat(driver: *mut c_void) -> *mut c_void;
}
static GET_TOKEN: Types<1> = Types([&TOKEN]);
#[allow(unused_unsafe)] // Taking an extern static address requires unsafe on Rust 1.80.
static ACTIVATE: Types<2> = Types([ptr::null(), unsafe { ptr::addr_of!(wl_surface_interface) }]);
#[allow(unused_unsafe)] // Rust 1.80.
static SERIAL: Types<2> = Types([ptr::null(), unsafe { ptr::addr_of!(wl_seat_interface) }]);
#[allow(unused_unsafe)] // Rust 1.80.
static SURFACE: Types<1> = Types([unsafe { ptr::addr_of!(wl_surface_interface) }]);
static STRING: Types<1> = Types([ptr::null()]);
static ACTIVATION_METHODS: [Message; 3] = [
    Message {
        name: c"destroy".as_ptr(),
        signature: c"".as_ptr(),
        types: ptr::null(),
    },
    Message {
        name: c"get_activation_token".as_ptr(),
        signature: c"n".as_ptr(),
        types: GET_TOKEN.0.as_ptr(),
    },
    Message {
        name: c"activate".as_ptr(),
        signature: c"so".as_ptr(),
        types: ACTIVATE.0.as_ptr(),
    },
];
static ACTIVATION: Interface = Interface {
    name: c"xdg_activation_v1".as_ptr(),
    version: 1,
    method_count: 3,
    methods: ACTIVATION_METHODS.as_ptr(),
    event_count: 0,
    events: ptr::null(),
};
static TOKEN_METHODS: [Message; 5] = [
    Message {
        name: c"set_serial".as_ptr(),
        signature: c"uo".as_ptr(),
        types: SERIAL.0.as_ptr(),
    },
    Message {
        name: c"set_app_id".as_ptr(),
        signature: c"s".as_ptr(),
        types: STRING.0.as_ptr(),
    },
    Message {
        name: c"set_surface".as_ptr(),
        signature: c"o".as_ptr(),
        types: SURFACE.0.as_ptr(),
    },
    Message {
        name: c"commit".as_ptr(),
        signature: c"".as_ptr(),
        types: ptr::null(),
    },
    Message {
        name: c"destroy".as_ptr(),
        signature: c"".as_ptr(),
        types: ptr::null(),
    },
];
static TOKEN_EVENTS: [Message; 1] = [Message {
    name: c"done".as_ptr(),
    signature: c"s".as_ptr(),
    types: STRING.0.as_ptr(),
}];
static TOKEN: Interface = Interface {
    name: c"xdg_activation_token_v1".as_ptr(),
    version: 1,
    method_count: 5,
    methods: TOKEN_METHODS.as_ptr(),
    event_count: 1,
    events: TOKEN_EVENTS.as_ptr(),
};

struct Globals {
    activation: Cell<*mut c_void>,
    name: Cell<Option<u32>>,
}
impl Default for Globals {
    fn default() -> Self {
        Self {
            activation: Cell::new(ptr::null_mut()),
            name: Cell::new(None),
        }
    }
}
#[repr(C)]
struct RegistryListener {
    global: unsafe extern "C" fn(*mut c_void, *mut c_void, u32, *const c_char, u32),
    removed: unsafe extern "C" fn(*mut c_void, *mut c_void, u32),
}
#[repr(C)]
struct TokenListener {
    done: unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_char),
}
#[repr(C)]
struct SyncListener {
    done: unsafe extern "C" fn(*mut c_void, *mut c_void, u32),
}
static REGISTRY_LISTENER: RegistryListener = RegistryListener { global, removed };
static TOKEN_LISTENER: TokenListener = TokenListener { done };
static SYNC_LISTENER: SyncListener = SyncListener { done: synced };
unsafe extern "C" fn global(
    data: *mut c_void,
    registry: *mut c_void,
    name: u32,
    interface: *const c_char,
    version: u32,
) {
    // SAFETY: the private queue owns this listener; its pinned Globals
    // allocation lives until the registry proxy is destroyed.
    unsafe {
        let state = &*data.cast::<Globals>();
        if version >= 1
            && CStr::from_ptr(interface) == c"xdg_activation_v1"
            && state.activation.get().is_null()
        {
            let object = wl_proxy_marshal_flags(
                registry,
                0,
                ptr::addr_of!(ACTIVATION).cast(),
                1,
                0,
                name,
                ACTIVATION.name,
                1u32,
                ptr::null_mut::<c_void>(),
            );
            state.activation.set(object);
            state.name.set(Some(name));
        }
    }
}
unsafe extern "C" fn removed(data: *mut c_void, _: *mut c_void, name: u32) {
    // SAFETY: same listener lifetime as global; child tokens survive the
    // activation object's destruction under the protocol contract.
    unsafe {
        let state = &*data.cast::<Globals>();
        if state.name.get() == Some(name) {
            let object = state.activation.replace(ptr::null_mut());
            if !object.is_null() {
                destroy(object, 0);
            }
            state.name.set(None);
        }
    }
}
unsafe extern "C" fn done(data: *mut c_void, _: *mut c_void, token: *const c_char) {
    // SAFETY: token reply storage lives through the private roundtrip;
    // the protocol string is copied before this callback returns.
    unsafe {
        *(*data.cast::<RefCell<Option<CString>>>()).borrow_mut() =
            Some(CStr::from_ptr(token).to_owned());
    }
}
unsafe extern "C" fn synced(data: *mut c_void, _: *mut c_void, _: u32) {
    // SAFETY: the private sync proxy is destroyed before its signal storage.
    unsafe {
        (*data.cast::<Cell<bool>>()).set(true);
    }
}
unsafe fn wait(connection: &Connection, ready: impl Fn() -> bool) -> Result<(), PresentError> {
    let end = Instant::now() + Duration::from_secs(2);
    // SAFETY: UI thread, one reader at a time on FLTK's connection. Only
    // this queue is dispatched; default-queue UI callbacks remain pending.
    // Every successful prepare is paired with read_events or cancel_read.
    unsafe {
        loop {
            if wl_display_dispatch_queue_pending(connection.display, connection.queue) < 0 {
                return Err(PresentError::Native(
                    "Wayland activation dispatch failed".into(),
                ));
            }
            if ready() {
                return Ok(());
            }
            let left = end
                .checked_duration_since(Instant::now())
                .ok_or(PresentError::TimedOut)?;
            if wl_display_prepare_read_queue(connection.display, connection.queue) != 0 {
                continue;
            }
            let flush = wl_display_flush(connection.display);
            let write = flush < 0
                && std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock;
            if flush < 0 && !write {
                wl_display_cancel_read(connection.display);
                return Err(PresentError::Native(
                    "Wayland activation flush failed".into(),
                ));
            }
            let mut fd = libc::pollfd {
                fd: wl_display_get_fd(connection.display),
                events: libc::POLLIN | if write { libc::POLLOUT } else { 0 },
                revents: 0,
            };
            let n = libc::poll(&mut fd, 1, left.as_millis().clamp(1, 2000) as i32);
            if n < 0 {
                wl_display_cancel_read(connection.display);
                if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(PresentError::Native(
                    "Wayland activation poll failed".into(),
                ));
            }
            if n == 0 {
                wl_display_cancel_read(connection.display);
                return Err(PresentError::TimedOut);
            }
            if fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                wl_display_cancel_read(connection.display);
                return Err(PresentError::Native(
                    "Wayland activation connection closed".into(),
                ));
            }
            if fd.revents & libc::POLLIN != 0 {
                if wl_display_read_events(connection.display) < 0 {
                    return Err(PresentError::Native(
                        "Wayland activation read failed".into(),
                    ));
                }
            } else {
                wl_display_cancel_read(connection.display);
            }
        }
    }
}
unsafe fn sync(connection: &Connection) -> Result<(), PresentError> {
    // SAFETY: wrapper-created callbacks inherit our private queue. The
    // server destroys its callback; we destroy the local proxy on all paths.
    unsafe {
        let wrapper = wl_proxy_create_wrapper(connection.display);
        if wrapper.is_null() {
            return Err(PresentError::Native("Wayland sync wrapper failed".into()));
        }
        wl_proxy_set_queue(wrapper, connection.queue);
        let callback = wl_proxy_marshal_flags(
            wrapper,
            0,
            ptr::addr_of!(wl_callback_interface).cast(),
            1,
            0,
            ptr::null_mut::<c_void>(),
        );
        wl_proxy_wrapper_destroy(wrapper);
        if callback.is_null() {
            return Err(PresentError::Native(
                "Wayland sync allocation failed".into(),
            ));
        }
        let signal = Cell::new(false);
        let result = if wl_proxy_add_listener(
            callback,
            &SYNC_LISTENER as *const SyncListener as *mut *const c_void,
            (&signal as *const Cell<bool> as *mut Cell<bool>).cast(),
        ) == 0
        {
            wait(connection, || signal.get())
        } else {
            Err(PresentError::Native("Wayland sync listener failed".into()))
        };
        wl_proxy_destroy(callback);
        result
    }
}
unsafe fn destroy(object: *mut c_void, opcode: u32) {
    unsafe {
        wl_proxy_marshal_flags(object, opcode, ptr::null(), 1, 1);
    }
}

struct Connection {
    display: *mut c_void,
    queue: *mut c_void,
    registry: *mut c_void,
    globals: Box<Globals>,
    identity: (u64, u64),
    discovered: Cell<bool>,
}
thread_local! {static CONNECTION:RefCell<Option<Connection>>=const{RefCell::new(None)};}
unsafe fn identity(display: *mut c_void) -> Option<(u64, u64)> {
    // SAFETY: the borrowed display is live; fstat does not take ownership
    // of its socket. The inode detects reconnects even with reused pointers.
    unsafe {
        let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
        if libc::fstat(wl_display_get_fd(display), info.as_mut_ptr()) != 0 {
            return None;
        }
        let info = info.assume_init();
        Some((info.st_dev, info.st_ino))
    }
}
impl Connection {
    unsafe fn new(display: *mut c_void) -> Result<Self, PresentError> {
        // SAFETY: all proxies use FLTK's UI-thread connection. Objects
        // created from a wrapper inherit only our private event queue.
        unsafe {
            let identity = identity(display).ok_or_else(|| {
                PresentError::Native("Wayland socket identity is unavailable".into())
            })?;
            let queue = wl_display_create_queue(display);
            if queue.is_null() {
                return Err(PresentError::Native(
                    "Wayland queue allocation failed".into(),
                ));
            }
            let wrapper = wl_proxy_create_wrapper(display);
            if wrapper.is_null() {
                wl_event_queue_destroy(queue);
                return Err(PresentError::Native(
                    "Wayland display wrapper allocation failed".into(),
                ));
            }
            wl_proxy_set_queue(wrapper, queue);
            let registry = wl_proxy_marshal_flags(
                wrapper,
                1,
                ptr::addr_of!(wl_registry_interface).cast(),
                wl_proxy_get_version(wrapper),
                0,
                ptr::null_mut::<c_void>(),
            );
            wl_proxy_wrapper_destroy(wrapper);
            if registry.is_null() {
                wl_event_queue_destroy(queue);
                return Err(PresentError::Native(
                    "Wayland registry allocation failed".into(),
                ));
            }
            let mut connection = Self {
                display,
                queue,
                registry,
                globals: Box::default(),
                identity,
                discovered: Cell::new(false),
            };
            let listener = &REGISTRY_LISTENER as *const RegistryListener as *mut *const c_void;
            if wl_proxy_add_listener(
                registry,
                listener,
                (&mut *connection.globals as *mut Globals).cast(),
            ) != 0
            {
                return Err(PresentError::Native(
                    "Wayland activation discovery failed".into(),
                ));
            }
            Ok(connection)
        }
    }
    unsafe fn live(&self) -> bool {
        unsafe { fl_wl_display() == self.display && identity(self.display) == Some(self.identity) }
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        // SAFETY: never touch proxies after FLTK disconnected/reconnected
        // its display. Otherwise destroy only the objects we created.
        unsafe {
            if self.live() {
                let object = self.globals.activation.get();
                if !object.is_null() {
                    if wl_display_get_error(self.display) == 0 {
                        destroy(object, 0);
                    } else {
                        wl_proxy_destroy(object);
                    }
                }
                wl_proxy_destroy(self.registry);
                wl_event_queue_destroy(self.queue);
            }
        }
    }
}
unsafe fn surface<W: WidgetExt + ?Sized>(window: &W) -> *mut c_void {
    // SAFETY: called only with shown live FLTK windows on this backend.
    unsafe {
        let Some(window) = window.as_window() else {
            return ptr::null_mut();
        };
        let xid = window.raw_handle();
        if xid.is_null() {
            ptr::null_mut()
        } else {
            fl_wl_surface(xid)
        }
    }
}
unsafe fn token(connection: &Connection) -> Result<CString, PresentError> {
    // SAFETY: private-queue listeners do not dispatch application events.
    // The stack reply stays live until its token proxy is destroyed.
    unsafe {
        let activation = connection.globals.activation.get();
        let object = wl_proxy_marshal_flags(
            activation,
            1,
            ptr::addr_of!(TOKEN).cast(),
            1,
            0,
            ptr::null_mut::<c_void>(),
        );
        if object.is_null() {
            return Err(PresentError::Native(
                "Wayland token allocation failed".into(),
            ));
        }
        let reply: RefCell<Option<CString>> = RefCell::new(None);
        let listener = &TOKEN_LISTENER as *const TokenListener as *mut *const c_void;
        if wl_proxy_add_listener(
            object,
            listener,
            (&reply as *const RefCell<Option<CString>> as *mut RefCell<Option<CString>>).cast(),
        ) != 0
        {
            destroy(object, 4);
            return Err(PresentError::Native("Wayland token listener failed".into()));
        }
        let driver = fl_screen_driver();
        if crate::focus::in_native_dispatch()
            && matches!(
                fltk::app::event(),
                fltk::enums::Event::Push
                    | fltk::enums::Event::Released
                    | fltk::enums::Event::KeyDown
                    | fltk::enums::Event::Shortcut
            )
        {
            let (serial, seat) = (get_serial(driver), get_seat(driver));
            if serial != 0 && !seat.is_null() {
                wl_proxy_marshal_flags(object, 0, ptr::null(), 1, 0, serial, seat);
            }
        }
        if let Some(window) = crate::focus::current().and_then(|w| w.top_window()) {
            let origin = surface(window.as_ref());
            if !origin.is_null() {
                wl_proxy_marshal_flags(object, 2, ptr::null(), 1, 0, origin);
            }
        }
        wl_proxy_marshal_flags(object, 3, ptr::null(), 1, 0);
        let result = wait(connection, || reply.borrow().is_some());
        if let Err(error) = result {
            if wl_display_get_error(connection.display) == 0 {
                destroy(object, 4);
            } else {
                wl_proxy_destroy(object);
            }
            return Err(error);
        }
        destroy(object, 4);
        reply.into_inner().ok_or_else(|| {
            PresentError::Native("Wayland compositor returned no activation token".into())
        })
    }
}

/// Permission is captured at API entry, before mapping can dispatch another
/// widget event. No later poll borrows the serial of a different gesture.
pub(super) struct InputContext {
    display: *mut c_void,
    identity: Option<(u64, u64)>,
    serial: u32,
    seat: *mut c_void,
    origin: Option<Box<dyn WindowExt>>,
}
impl InputContext {
    pub(super) fn empty() -> Self {
        Self {
            display: ptr::null_mut(),
            identity: None,
            serial: 0,
            seat: ptr::null_mut(),
            origin: None,
        }
    }

    pub(super) fn capture() -> Self {
        // SAFETY: the portable facade calls on the FLTK UI thread. Native
        // pointers are only compared after yielding, never dereferenced until
        // the display and the current seat have been checked again.
        unsafe {
            let display = fl_wl_display();
            if display.is_null() || wl_display_get_error(display) != 0 {
                return Self::empty();
            }
            let Some(identity) = identity(display) else {
                return Self::empty();
            };
            let mut input = Self {
                display,
                identity: Some(identity),
                ..Self::empty()
            };
            if crate::focus::in_native_dispatch()
                && matches!(
                    fltk::app::event(),
                    fltk::enums::Event::Push
                        | fltk::enums::Event::Released
                        | fltk::enums::Event::KeyDown
                        | fltk::enums::Event::Shortcut
                )
            {
                let driver = fl_screen_driver();
                if !driver.is_null() {
                    input.serial = get_serial(driver);
                    if input.serial != 0 {
                        input.seat = get_seat(driver);
                    }
                }
            }
            input.origin = crate::focus::current()
                .filter(|w| !w.was_deleted())
                .and_then(|w| w.top_window());
            input
        }
    }

    unsafe fn apply(&self, connection: &Connection, object: *mut c_void) {
        // SAFETY: connection.live was checked in this poll, and no application
        // callbacks run while its private queue is dispatched. A seat pointer
        // captured before yielding is not an owned proxy; only use it when
        // FLTK still reports that exact live seat on the same connection.
        unsafe {
            if self.display != connection.display || self.identity != Some(connection.identity) {
                return;
            }
            let driver = fl_screen_driver();
            if !driver.is_null()
                && self.permits_serial(connection.display, connection.identity, get_seat(driver))
            {
                wl_proxy_marshal_flags(object, 0, ptr::null(), 1, 0, self.serial, self.seat);
            }
            if let Some(origin) = self
                .origin
                .as_ref()
                .filter(|w| !w.was_deleted() && w.shown() && w.parent().is_none())
            {
                // The wrapper tracks deletion. Resolve its current surface
                // now rather than retaining a raw wl_surface across a yield.
                let origin = surface(origin.as_ref());
                if !origin.is_null() {
                    wl_proxy_marshal_flags(object, 2, ptr::null(), 1, 0, origin);
                }
            }
        }
    }

    fn permits_serial(
        &self,
        display: *mut c_void,
        identity: (u64, u64),
        seat: *mut c_void,
    ) -> bool {
        self.display == display
            && self.identity == Some(identity)
            && self.serial != 0
            && !self.seat.is_null()
            && self.seat == seat
    }
}

const PHASE_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_TOKEN_BYTES: usize = 60 * 1024;

struct SyncPending {
    object: *mut c_void,
    ready: Box<Cell<bool>>,
}
struct TokenPending {
    object: *mut c_void,
    reply: Box<RefCell<Option<Result<CString, PresentError>>>>,
}
enum Phase {
    Prepare,
    Discovery(SyncPending),
    Token(TokenPending),
}
static ASYNC_TOKEN_LISTENER: TokenListener = TokenListener { done: async_done };
unsafe extern "C" fn async_done(data: *mut c_void, _: *mut c_void, token: *const c_char) {
    // SAFETY: the boxed reply is stable until the token proxy is retired;
    // cancellation destroys that proxy before freeing this allocation.
    unsafe {
        let value = CStr::from_ptr(token);
        let result = if value.to_bytes().len() <= MAX_TOKEN_BYTES {
            Ok(value.to_owned())
        } else {
            Err(PresentError::Native(
                "Wayland activation token exceeds the supported limit".into(),
            ))
        };
        *(*data.cast::<RefCell<Option<Result<CString, PresentError>>>>()).borrow_mut() =
            Some(result);
    }
}

fn expired(started: Instant, now: Instant) -> bool {
    now.saturating_duration_since(started) >= PHASE_TIMEOUT
}

unsafe fn flush(connection: &Connection) -> Result<bool, PresentError> {
    // SAFETY: libwayland's connection socket is nonblocking. EAGAIN means
    // output remains queued, not that this call should wait for POLLOUT.
    unsafe {
        if wl_display_flush(connection.display) >= 0 {
            return Ok(false);
        }
        match std::io::Error::last_os_error().kind() {
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted => Ok(true),
            _ => Err(PresentError::Native(
                "Wayland activation flush failed".into(),
            )),
        }
    }
}

/// Run one protocol pass without waiting for the compositor. FLTK owns this
/// display and all reads occur on its UI thread. Every prepare is paired with
/// read_events or cancel_read before returning to the application's loop.
unsafe fn pump(connection: &Connection) -> Result<(), PresentError> {
    unsafe {
        if wl_display_dispatch_queue_pending(connection.display, connection.queue) < 0 {
            return Err(PresentError::Native(
                "Wayland activation dispatch failed".into(),
            ));
        }
        if wl_display_prepare_read_queue(connection.display, connection.queue) != 0 {
            // A pending event or interruption is handled on the next UI turn;
            // retrying in a loop here would turn a poll into a busy wait.
            return if wl_display_get_error(connection.display) == 0 {
                Ok(())
            } else {
                Err(PresentError::Native(
                    "Wayland activation prepare failed".into(),
                ))
            };
        }
        struct Prepared {
            display: *mut c_void,
            armed: bool,
        }
        impl Drop for Prepared {
            fn drop(&mut self) {
                if self.armed {
                    // SAFETY: this guard only exists after a successful
                    // prepare, on the same UI thread and connection.
                    unsafe { wl_display_cancel_read(self.display) }
                }
            }
        }
        let mut prepared = Prepared {
            display: connection.display,
            armed: true,
        };
        let write = flush(connection)?;
        let mut fd = libc::pollfd {
            fd: wl_display_get_fd(connection.display),
            events: libc::POLLIN | if write { libc::POLLOUT } else { 0 },
            revents: 0,
        };
        let n = libc::poll(&mut fd, 1, 0);
        if n < 0 {
            return if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                Ok(())
            } else {
                Err(PresentError::Native(
                    "Wayland activation poll failed".into(),
                ))
            };
        }
        if n == 0 {
            return Ok(());
        }
        if fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            return Err(PresentError::Native(
                "Wayland activation connection closed".into(),
            ));
        }
        if fd.revents & libc::POLLIN != 0 {
            prepared.armed = false;
            if wl_display_read_events(connection.display) < 0 {
                let transient = matches!(
                    std::io::Error::last_os_error().kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                );
                return if transient && wl_display_get_error(connection.display) == 0 {
                    Ok(())
                } else {
                    Err(PresentError::Native(
                        "Wayland activation read failed".into(),
                    ))
                };
            }
        } else {
            // Cancel the read intent before a second flush. No prepared read
            // may leak into FLTK's next event-dispatch iteration.
            wl_display_cancel_read(connection.display);
            prepared.armed = false;
        }
        if write && fd.revents & libc::POLLOUT != 0 {
            let _ = flush(connection)?;
        }
        if wl_display_dispatch_queue_pending(connection.display, connection.queue) < 0 {
            return Err(PresentError::Native(
                "Wayland activation dispatch failed".into(),
            ));
        }
        Ok(())
    }
}

/// The request temporarily owns the cached private connection. The facade
/// permits one asynchronous presentation per UI thread and excludes the
/// synchronous API until this request completes or is dropped.
pub(in crate::platform::presentation) struct Request {
    connection: Option<Connection>,
    input: Option<InputContext>,
    hint: Option<CString>,
    phase: Phase,
    started: Instant,
    finished: Option<Result<Presentation, PresentError>>,
}
impl Request {
    pub(super) fn begin<W: WindowExt + ?Sized>(
        window: &W,
        options: PresentOptions<'_>,
        input: InputContext,
    ) -> Result<Self, PresentError> {
        // SAFETY: the facade supplies a mapped own window on the UI thread.
        // Taking the cache out avoids retaining a RefCell borrow across UI
        // turns; callback userdata resides in stable heap allocations.
        unsafe {
            let display = fl_wl_display();
            if display.is_null() || surface(window).is_null() {
                return Err(PresentError::Native("missing Wayland surface".into()));
            }
            if wl_display_get_error(display) != 0 {
                return Err(PresentError::Native("Wayland display has an error".into()));
            }
            let hint = options
                .activation_token
                .filter(|s| !s.is_empty())
                .or(options.startup_id.filter(|s| !s.is_empty()));
            let hint = hint
                .map(|s| {
                    if s.len() > MAX_TOKEN_BYTES {
                        Err(PresentError::InvalidHint)
                    } else {
                        CString::new(s).map_err(|_| PresentError::InvalidHint)
                    }
                })
                .transpose()?;
            let connection = CONNECTION.with(|slot| {
                let mut slot = slot.try_borrow_mut().map_err(|_| PresentError::Busy)?;
                let connection = slot.take();
                match connection {
                    Some(connection) if connection.live() => Ok(connection),
                    _ => Connection::new(display),
                }
            })?;
            let mut request = Self {
                connection: Some(connection),
                input: Some(input),
                hint,
                phase: Phase::Prepare,
                started: Instant::now(),
                finished: None,
            };
            let c = request.connection.as_ref().unwrap();
            if !c.discovered.get() {
                request.phase = Phase::Discovery(start_sync(c)?);
            }
            let _ = flush(c)?;
            Ok(request)
        }
    }

    pub(super) fn poll<W: WindowExt + ?Sized>(
        &mut self,
        window: &W,
    ) -> Poll<Result<Presentation, PresentError>> {
        if let Some(result) = &self.finished {
            return Poll::Ready(result.clone());
        }
        let Some(connection) = self.connection.take() else {
            return Poll::Ready(Err(PresentError::Native(
                "Wayland activation request has no connection".into(),
            )));
        };
        // SAFETY: the cache connection is exclusively leased to this request.
        // Its identity is checked before any proxy access on each UI turn.
        let result = unsafe {
            if !connection.live() {
                Err(PresentError::Native(
                    "Wayland activation connection changed".into(),
                ))
            } else if wl_display_get_error(connection.display) != 0 {
                Err(PresentError::Native("Wayland display has an error".into()))
            } else {
                self.advance(&connection, window)
            }
        };
        self.connection = Some(connection);
        match result {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(value)) => self.finish(Ok(value)),
            Err(error) => self.finish(Err(error)),
        }
    }

    unsafe fn advance<W: WindowExt + ?Sized>(
        &mut self,
        connection: &Connection,
        window: &W,
    ) -> Result<Poll<Presentation>, PresentError> {
        unsafe {
            if expired(self.started, Instant::now()) {
                return Err(PresentError::TimedOut);
            }
            pump(connection)?;
            if expired(self.started, Instant::now()) {
                return Err(PresentError::TimedOut);
            }
            match &self.phase {
                Phase::Discovery(sync) if !sync.ready.get() => return Ok(Poll::Pending),
                Phase::Discovery(_) => {
                    self.retire_phase(connection);
                    connection.discovered.set(true);
                    self.started = Instant::now();
                }
                Phase::Token(token) => {
                    let value = token.reply.borrow_mut().take();
                    let Some(value) = value else {
                        return Ok(Poll::Pending);
                    };
                    self.retire_phase(connection);
                    return self.activate(connection, window, &value?).map(Poll::Ready);
                }
                Phase::Prepare => {}
            }
            if connection.globals.activation.get().is_null() {
                return Ok(Poll::Ready(unsupported()));
            }
            if let Some(hint) = self.hint.take() {
                self.input = None;
                return self.activate(connection, window, &hint).map(Poll::Ready);
            }
            self.phase = Phase::Token(start_token(connection, self.input.take().as_ref())?);
            self.started = Instant::now();
            let _ = flush(connection)?;
            Ok(Poll::Pending)
        }
    }

    unsafe fn activate<W: WindowExt + ?Sized>(
        &self,
        connection: &Connection,
        window: &W,
        value: &CStr,
    ) -> Result<Presentation, PresentError> {
        unsafe {
            let activation = connection.globals.activation.get();
            if activation.is_null() {
                return Ok(unsupported());
            }
            let target = surface(window);
            if target.is_null() {
                return Err(PresentError::Native("missing Wayland surface".into()));
            }
            wl_proxy_marshal_flags(activation, 2, ptr::null(), 1, 0, value.as_ptr(), target);
            let _ = flush(connection)?;
            Ok(Presentation {
                backend: PresentBackend::Wayland,
                status: PresentStatus::Requested,
            })
        }
    }

    unsafe fn retire_phase(&mut self, connection: &Connection) {
        // SAFETY: destroy proxies before their boxed listener userdata drops.
        // A disconnected FLTK display already invalidated its objects; never
        // dereference their old pointers in that case.
        unsafe {
            let phase = std::mem::replace(&mut self.phase, Phase::Prepare);
            if connection.live() {
                match &phase {
                    Phase::Discovery(sync) => wl_proxy_destroy(sync.object),
                    Phase::Token(token) => {
                        if wl_display_get_error(connection.display) == 0 {
                            destroy(token.object, 4);
                        } else {
                            wl_proxy_destroy(token.object);
                        }
                    }
                    Phase::Prepare => {}
                }
            }
        }
    }

    fn finish(
        &mut self,
        result: Result<Presentation, PresentError>,
    ) -> Poll<Result<Presentation, PresentError>> {
        self.release();
        self.finished = Some(result.clone());
        Poll::Ready(result)
    }

    fn release(&mut self) {
        self.input = None;
        self.hint = None;
        if let Some(connection) = self.connection.take() {
            // SAFETY: same UI-thread lifetime as begin/poll. Returning an
            // undiscovered connection after cancellation requires the next
            // request to perform its own sync; no old signal is reused.
            unsafe { self.retire_phase(&connection) };
            // A caller may retain its handle in TLS that was initialized
            // before this cache. During thread exit the cache can already be
            // destroyed; then drop our connection instead of caching it.
            let _ = CONNECTION.try_with(|slot| {
                if let Ok(mut slot) = slot.try_borrow_mut() {
                    if slot.is_none() && unsafe { connection.live() } {
                        *slot = Some(connection);
                    }
                }
            });
        }
    }
}
impl Drop for Request {
    fn drop(&mut self) {
        self.release();
    }
}

fn unsupported() -> Presentation {
    Presentation {
        backend: PresentBackend::Wayland,
        status: PresentStatus::Unsupported,
    }
}

unsafe fn start_sync(connection: &Connection) -> Result<SyncPending, PresentError> {
    unsafe {
        let wrapper = wl_proxy_create_wrapper(connection.display);
        if wrapper.is_null() {
            return Err(PresentError::Native("Wayland sync wrapper failed".into()));
        }
        wl_proxy_set_queue(wrapper, connection.queue);
        let object = wl_proxy_marshal_flags(
            wrapper,
            0,
            ptr::addr_of!(wl_callback_interface).cast(),
            1,
            0,
            ptr::null_mut::<c_void>(),
        );
        wl_proxy_wrapper_destroy(wrapper);
        if object.is_null() {
            return Err(PresentError::Native(
                "Wayland sync allocation failed".into(),
            ));
        }
        let mut pending = SyncPending {
            object,
            ready: Box::new(Cell::new(false)),
        };
        if wl_proxy_add_listener(
            object,
            &SYNC_LISTENER as *const SyncListener as *mut *const c_void,
            (&mut *pending.ready as *mut Cell<bool>).cast(),
        ) != 0
        {
            wl_proxy_destroy(object);
            return Err(PresentError::Native("Wayland sync listener failed".into()));
        }
        Ok(pending)
    }
}

unsafe fn start_token(
    connection: &Connection,
    input: Option<&InputContext>,
) -> Result<TokenPending, PresentError> {
    unsafe {
        let object = wl_proxy_marshal_flags(
            connection.globals.activation.get(),
            1,
            ptr::addr_of!(TOKEN).cast(),
            1,
            0,
            ptr::null_mut::<c_void>(),
        );
        if object.is_null() {
            return Err(PresentError::Native(
                "Wayland token allocation failed".into(),
            ));
        }
        let mut pending = TokenPending {
            object,
            reply: Box::new(RefCell::new(None)),
        };
        if wl_proxy_add_listener(
            object,
            &ASYNC_TOKEN_LISTENER as *const TokenListener as *mut *const c_void,
            (&mut *pending.reply as *mut RefCell<Option<Result<CString, PresentError>>>).cast(),
        ) != 0
        {
            destroy(object, 4);
            return Err(PresentError::Native("Wayland token listener failed".into()));
        }
        if let Some(input) = input {
            input.apply(connection, object);
        }
        wl_proxy_marshal_flags(object, 3, ptr::null(), 1, 0);
        Ok(pending)
    }
}

pub(super) fn present<W: WindowExt + ?Sized>(
    window: &W,
    options: PresentOptions<'_>,
) -> Result<Presentation, PresentError> {
    // SAFETY: this backend is selected only for a live Wayland display;
    // the portable facade checked UI thread and top-level window lifetime.
    unsafe {
        let display = fl_wl_display();
        let target = surface(window);
        if target.is_null() {
            return Err(PresentError::Native("missing Wayland surface".into()));
        }
        if wl_display_get_error(display) != 0 {
            return Err(PresentError::Native("Wayland display has an error".into()));
        }
        CONNECTION.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.as_ref().map_or(true, |c| !c.live()) {
                *slot = Some(Connection::new(display)?);
            }
            let c = slot.as_ref().unwrap();
            if !c.discovered.get() {
                sync(c)?;
                c.discovered.set(true);
            }
            if wl_display_dispatch_queue_pending(display, c.queue) < 0 {
                return Err(PresentError::Native(
                    "Wayland activation queue failed".into(),
                ));
            }
            let hint = options
                .activation_token
                .filter(|s| !s.is_empty())
                .or(options.startup_id.filter(|s| !s.is_empty()));
            if c.globals.activation.get().is_null() {
                return Ok(Presentation {
                    backend: PresentBackend::Wayland,
                    status: PresentStatus::Unsupported,
                });
            }
            let token = match hint {
                Some(value) => CString::new(value).map_err(|_| PresentError::InvalidHint)?,
                None => token(c)?,
            };
            let activation = c.globals.activation.get();
            if activation.is_null() {
                return Ok(Presentation {
                    backend: PresentBackend::Wayland,
                    status: PresentStatus::Unsupported,
                });
            }
            wl_proxy_marshal_flags(activation, 2, ptr::null(), 1, 0, token.as_ptr(), target);
            if wl_display_flush(display) < 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::WouldBlock
            {
                return Err(PresentError::Native(
                    "Wayland activation flush failed".into(),
                ));
            }
            Ok(Presentation {
                backend: PresentBackend::Wayland,
                status: PresentStatus::Requested,
            })
        })
    }
}

#[cfg(test)]
mod async_tests {
    use super::*;

    #[test]
    fn phase_deadline_expires_at_the_bound_and_resets_for_token_acquisition() {
        let begin = Instant::now();
        assert!(!expired(
            begin,
            begin + PHASE_TIMEOUT - Duration::from_nanos(1)
        ));
        assert!(expired(begin, begin + PHASE_TIMEOUT));
        let token_begin = begin + Duration::from_millis(1900);
        assert!(!expired(token_begin, begin + PHASE_TIMEOUT));
        assert!(expired(token_begin, token_begin + PHASE_TIMEOUT));
    }

    #[test]
    fn native_permission_is_not_borrowed_after_a_seat_or_display_replacement() {
        // These addresses are identity values only; this test never calls a
        // native API or dereferences them.
        let display = 1usize as *mut c_void;
        let seat = 2usize as *mut c_void;
        let other = 3usize as *mut c_void;
        let mut input = InputContext {
            display,
            identity: Some((5, 7)),
            serial: 11,
            seat,
            origin: None,
        };
        assert!(input.permits_serial(display, (5, 7), seat));
        assert!(!input.permits_serial(display, (5, 8), seat));
        assert!(!input.permits_serial(other, (5, 7), seat));
        assert!(!input.permits_serial(display, (5, 7), other));
        input.serial = 0;
        assert!(!input.permits_serial(display, (5, 7), seat));
        assert!(!InputContext::empty().permits_serial(display, (5, 7), seat));
    }

    #[test]
    fn token_reply_is_owned_bounded_and_does_not_copy_rejected_context_into_errors() {
        let mut reply: Box<RefCell<Option<Result<CString, PresentError>>>> =
            Box::new(RefCell::new(None));
        let storage = (&mut *reply as *mut RefCell<Option<Result<CString, PresentError>>>).cast();
        let ordinary = CString::new("private-context").unwrap();
        unsafe { async_done(storage, ptr::null_mut(), ordinary.as_ptr()) };
        assert_eq!(reply.borrow_mut().take().unwrap().unwrap(), ordinary);
        let limit = CString::new(vec![b'x'; MAX_TOKEN_BYTES]).unwrap();
        unsafe { async_done(storage, ptr::null_mut(), limit.as_ptr()) };
        assert_eq!(reply.borrow_mut().take().unwrap().unwrap(), limit);
        let oversized = CString::new(vec![b'z'; MAX_TOKEN_BYTES + 1]).unwrap();
        unsafe { async_done(storage, ptr::null_mut(), oversized.as_ptr()) };
        let error = reply.borrow_mut().take().unwrap().unwrap_err();
        assert!(matches!(error, PresentError::Native(_)));
        assert!(!error.to_string().contains("zzzz"));
        let empty = CString::new("").unwrap();
        unsafe { async_done(storage, ptr::null_mut(), empty.as_ptr()) };
        assert_eq!(reply.borrow_mut().take().unwrap().unwrap(), empty);
    }
}
