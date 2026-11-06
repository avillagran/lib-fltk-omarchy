//! OS side of `crate::media` playback (feature `playback`): where the
//! system ffmpeg lives, how its child processes are tied to this process,
//! and which audio outputs to try on this OS. The player itself is
//! portable: it builds ffmpeg command lines and reads pipes.
//!
//! | | Linux / BSD | macOS | Windows |
//! |---|---|---|---|
//! | Tools searched | `PATH` | `PATH`, Homebrew, MacPorts | `PATH`, WinGet links, Scoop, Chocolatey |
//! | Child dies with the app | `PR_SET_PDEATHSIG` (Linux), exit hook | exit hook; video ends on its broken pipe | job object, kill on close |
//! | Audio outputs, in order | ffmpeg `pulse`, `pw-play`, `paplay`, ffmpeg `alsa`, `aplay`, `ffplay` | ffmpeg `audiotoolbox`, `ffplay` | `ffplay` |
//!
//! Every output that fails (missing, no server, no device) is skipped for
//! the rest of the run; with none left the video plays without sound and
//! says so.

use std::path::PathBuf;
use std::process::{Child, Command};

/// One way to make sound from the PCM the player decodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AudioOut {
    /// An ffmpeg output device (`-f <format> <device>`): one process, no
    /// PCM through a pipe.
    Device {
        format: &'static str,
        device: &'static str,
    },
    /// A player reading s16le 48 kHz stereo on stdin, fed by ffmpeg's
    /// stdout (child to child: the PCM never passes through this process).
    Player {
        program: &'static str,
        args: &'static [&'static str],
    },
}

impl AudioOut {
    /// Short name (for `OMARCHY_FLTK_AUDIO_OUT` and diagnostics).
    pub(crate) fn name(&self) -> &'static str {
        match self {
            AudioOut::Device { format, .. } => format,
            AudioOut::Player { program, .. } => program,
        }
    }
}

const PW_PLAY: AudioOut = AudioOut::Player {
    program: "pw-play",
    args: &[
        "--raw",
        "--format",
        "s16",
        "--rate",
        "48000",
        "--channels",
        "2",
        "-",
    ],
};
const PAPLAY: AudioOut = AudioOut::Player {
    program: "paplay",
    args: &["--raw", "--format=s16le", "--rate=48000", "--channels=2"],
};
const APLAY: AudioOut = AudioOut::Player {
    program: "aplay",
    args: &[
        "-q", "-t", "raw", "-f", "S16_LE", "-r", "48000", "-c", "2", "-",
    ],
};
/// Ships with most ffmpeg builds (Windows, Homebrew, Linux distributions).
const FFPLAY: AudioOut = AudioOut::Player {
    program: "ffplay",
    args: &[
        "-hide_banner",
        "-loglevel",
        "error",
        "-nodisp",
        "-autoexit",
        "-f",
        "s16le",
        "-sample_rate",
        "48000",
        "-ch_layout",
        "stereo",
        "-i",
        "-",
    ],
};

/// Audio outputs to try, most preferred first. The first is the lightest:
/// ffmpeg's own output device where the OS has one ffmpeg can drive
/// (PulseAudio protocol, also served by PipeWire; Core Audio), then
/// players fed by a pipe, then `ffplay`.
pub(crate) fn audio_outputs() -> &'static [AudioOut] {
    #[cfg(target_os = "macos")]
    {
        const OUTS: &[AudioOut] = &[
            AudioOut::Device {
                format: "audiotoolbox",
                device: "-",
            },
            FFPLAY,
        ];
        OUTS
    }
    #[cfg(target_os = "windows")]
    {
        // ffmpeg has no Windows audio output device; ffplay (SDL) does.
        const OUTS: &[AudioOut] = &[FFPLAY];
        OUTS
    }
    #[cfg(target_os = "linux")]
    {
        const OUTS: &[AudioOut] = &[
            AudioOut::Device {
                format: "pulse",
                device: "default",
            },
            PW_PLAY,
            PAPLAY,
            AudioOut::Device {
                format: "alsa",
                device: "default",
            },
            APLAY,
            FFPLAY,
        ];
        OUTS
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        const OUTS: &[AudioOut] = &[
            AudioOut::Device {
                format: "pulse",
                device: "default",
            },
            PAPLAY,
            FFPLAY,
        ];
        OUTS
    }
}

/// Every audio output known on any OS (`OMARCHY_FLTK_AUDIO_OUT=<name>`
/// forces one, for tests and diagnostics).
pub(crate) fn audio_out_named(name: &str) -> Option<AudioOut> {
    let all = [
        AudioOut::Device {
            format: "pulse",
            device: "default",
        },
        AudioOut::Device {
            format: "alsa",
            device: "default",
        },
        AudioOut::Device {
            format: "audiotoolbox",
            device: "-",
        },
        PW_PLAY,
        PAPLAY,
        APLAY,
        FFPLAY,
    ];
    all.into_iter().find(|o| o.name() == name)
}

/// Directories searched for `ffmpeg`, `ffprobe` and the audio players:
/// `PATH`, then where package managers install them (apps started from
/// the macOS Finder or the Windows Start menu get a short `PATH`).
pub(crate) fn tool_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    #[cfg(target_os = "macos")]
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"].map(PathBuf::from));
    #[cfg(target_os = "windows")]
    {
        let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
        if let Some(d) = env("LOCALAPPDATA") {
            dirs.push(d.join("Microsoft").join("WinGet").join("Links"));
        }
        if let Some(d) = env("USERPROFILE") {
            dirs.push(d.join("scoop").join("shims"));
        }
        if let Some(d) = env("ChocolateyInstall") {
            dirs.push(d.join("bin"));
        }
        if let Some(d) = env("ProgramFiles") {
            dirs.push(d.join("ffmpeg").join("bin"));
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    dirs.extend(["/usr/bin", "/usr/local/bin"].map(PathBuf::from));
    dirs
}

/// Environment for ffmpeg children. glibc (Linux): two malloc arenas and
/// a fixed 1 MiB mmap threshold, so freed multi-MB frames go back to the
/// OS instead of staying in per-thread arenas (measured on a native 1080p
/// stream: 216 MB PSS by default, 130 MB with these). Elsewhere nothing.
pub(crate) fn child_env() -> &'static [(&'static str, &'static str)] {
    if cfg!(all(target_os = "linux", target_env = "gnu")) {
        &[
            ("MALLOC_ARENA_MAX", "2"),
            ("MALLOC_MMAP_THRESHOLD_", "1048576"),
        ]
    } else {
        &[]
    }
}

/// File name of a tool on this OS (`ffmpeg.exe` on Windows).
pub(crate) fn exe_name(tool: &str) -> String {
    if cfg!(windows) {
        format!("{tool}.exe")
    } else {
        tool.to_string()
    }
}

/// Prepares a child so it cannot outlive this process and shows nothing
/// of its own: Linux asks the kernel to kill it when the spawning thread
/// (the one that also reaps it) goes away; every Unix disables core
/// dumps; Windows starts it without a console window.
pub(crate) fn contain(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        #[cfg(target_os = "linux")]
        // SAFETY: getpid is async-signal-safe and called before fork.
        let parent = unsafe { libc::getpid() };
        // SAFETY: only async-signal-safe calls (prctl, getppid, setrlimit)
        // between fork and exec.
        unsafe {
            cmd.pre_exec(move || {
                #[cfg(target_os = "linux")]
                {
                    if libc::prctl(
                        libc::PR_SET_PDEATHSIG,
                        libc::SIGKILL as libc::c_ulong,
                        0,
                        0,
                        0,
                    ) != 0
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                    // The parent died between fork and prctl.
                    if libc::getppid() != parent {
                        return Err(std::io::Error::other("parent exited"));
                    }
                }
                let none = libc::rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                };
                libc::setrlimit(libc::RLIMIT_CORE, &none);
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

/// Right after spawning: Windows puts the child in a job that kills its
/// processes when this process ends (its last handle closes). Elsewhere
/// nothing to do.
pub(crate) fn adopt(child: &Child) {
    #[cfg(windows)]
    win::assign(child);
    #[cfg(not(windows))]
    let _ = child;
}

/// Runs `f` when the process exits normally (`exit`, a return from
/// `main`) even if no destructor runs. Unix: `atexit`; Windows relies on
/// the job object instead. Returns true when registered.
pub(crate) fn at_exit(f: extern "C" fn()) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: registers a plain function pointer with the C runtime.
        unsafe { libc::atexit(f) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = f;
        false
    }
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;
    use std::sync::OnceLock;

    #[repr(C)]
    #[derive(Default)]
    struct BasicLimits {
        per_process_user_time: i64,
        per_job_user_time: i64,
        limit_flags: u32,
        min_working_set: usize,
        max_working_set: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimits {
        basic: BasicLimits,
        io: [u64; 6],
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory: usize,
        peak_job_memory: usize,
    }

    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;
    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attributes: *mut c_void, name: *const u16) -> *mut c_void;
        fn SetInformationJobObject(
            job: *mut c_void,
            class: i32,
            info: *mut c_void,
            len: u32,
        ) -> i32;
        fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
    }

    struct Job(usize);

    fn job() -> Option<*mut c_void> {
        static JOB: OnceLock<Job> = OnceLock::new();
        let j = JOB.get_or_init(|| {
            // SAFETY: plain Win32 calls with valid pointers; the handle is
            // kept for the life of the process (closing it kills the job).
            unsafe {
                let h = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
                if h.is_null() {
                    return Job(0);
                }
                let mut info = ExtendedLimits::default();
                info.basic.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    h,
                    JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                    (&mut info as *mut ExtendedLimits).cast(),
                    std::mem::size_of::<ExtendedLimits>() as u32,
                );
                Job(if ok != 0 { h as usize } else { 0 })
            }
        });
        (j.0 != 0).then_some(j.0 as *mut c_void)
    }

    pub(super) fn assign(child: &std::process::Child) {
        if let Some(j) = job() {
            // SAFETY: both handles are valid; failure leaves the child
            // outside the job (it still stops with its pipes).
            unsafe { AssignProcessToJobObject(j, child.as_raw_handle() as *mut c_void) };
        }
    }
}
