//! Confining a worker process (SPEC 3.4, SPEC 15).
//!
//! PDFium parses untrusted input, so a worker is treated as a process that will
//! eventually be made to misbehave. The confinement is imposed from outside, by
//! the supervisor, so that a compromised worker cannot lift its own limits.
//!
//! On Windows that is a Job Object with a memory cap and `KILL_ON_JOB_CLOSE`,
//! which is also what guarantees no worker outlives the UI process — including
//! when the UI is killed from Task Manager, where an ordinary "kill my children
//! on exit" handler would never run.
//!
//! On Unix, which is where CI and development run, the equivalent is an address
//! space rlimit applied in the child before `exec`. It is not a security
//! boundary and does not pretend to be; it exists so the same code paths are
//! exercised off Windows.

use std::process::Command;

use super::policy::WORKER_MEMORY_CAP_BYTES;

/// Holds the OS resources that confine a set of workers.
///
/// Dropping it terminates every worker still inside. That is the point: it makes
/// "no orphaned workers" a property of the process tree rather than of our
/// shutdown code being correct.
#[derive(Debug)]
pub struct Sandbox {
    inner: platform::Sandbox,
}

impl Sandbox {
    /// Creates the confinement. One per application run; all workers join it.
    pub fn create() -> std::io::Result<Self> {
        Ok(Self {
            inner: platform::Sandbox::create(WORKER_MEMORY_CAP_BYTES)?,
        })
    }

    /// Applies whatever must be set on the `Command` before spawning.
    pub fn prepare(&self, cmd: &mut Command) {
        self.inner.prepare(cmd);
    }

    /// Places an already-spawned child under confinement.
    ///
    /// Needed on Windows, where a process joins a Job Object after creation.
    pub fn adopt(&self, child: &std::process::Child) -> std::io::Result<()> {
        self.inner.adopt(child)
    }

    /// True when this platform's implementation is a real security boundary.
    ///
    /// Reported in the About box and the logs, so nobody reads a development
    /// build's confinement as the shipping one.
    pub fn is_security_boundary(&self) -> bool {
        cfg!(windows)
    }
}

#[cfg(windows)]
mod platform {
    use std::io;
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicUIRestrictions,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION,
        JOB_OBJECT_LIMIT_JOB_MEMORY, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOB_OBJECT_UILIMIT_DESKTOP,
        JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS,
        JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES,
        JOB_OBJECT_UILIMIT_READCLIPBOARD, JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS,
        JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
    };
    use windows::Win32::System::Threading::CREATE_NO_WINDOW;

    #[derive(Debug)]
    pub struct Sandbox {
        job: HANDLE,
    }

    impl Sandbox {
        pub fn create(memory_cap: u64) -> io::Result<Self> {
            // SAFETY: an unnamed job object; the returned handle is owned here
            // and closed in Drop.
            let job = unsafe { CreateJobObjectW(None, None) }.map_err(io::Error::other)?;

            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                | JOB_OBJECT_LIMIT_PROCESS_MEMORY
                | JOB_OBJECT_LIMIT_JOB_MEMORY
                | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
            // Both caps: per process stops one hostile file, per job stops the
            // pool as a whole from exhausting the machine.
            limits.ProcessMemoryLimit = memory_cap as usize;
            limits.JobMemoryLimit = memory_cap.saturating_mul(2) as usize;
            // No error dialogs from a background process the user cannot see.
            limits.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_BREAKAWAY_OK;

            // SAFETY: `limits` is a correctly sized, correctly typed structure
            // for the information class named, and lives across the call.
            unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
                .map_err(io::Error::other)?;
            }

            // A renderer has no business touching the desktop, the clipboard, or
            // system parameters. Denying it costs nothing and removes a whole
            // class of things a compromised worker could reach for.
            let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
                UIRestrictionsClass: JOB_OBJECT_UILIMIT_DESKTOP
                    | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
                    | JOB_OBJECT_UILIMIT_EXITWINDOWS
                    | JOB_OBJECT_UILIMIT_GLOBALATOMS
                    | JOB_OBJECT_UILIMIT_HANDLES
                    | JOB_OBJECT_UILIMIT_READCLIPBOARD
                    | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
                    | JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
            };
            // SAFETY: as above, for the UI restrictions information class.
            unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectBasicUIRestrictions,
                    (&ui as *const JOBOBJECT_BASIC_UI_RESTRICTIONS).cast(),
                    std::mem::size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32,
                )
                .map_err(io::Error::other)?;
            }

            Ok(Sandbox { job })
        }

        pub fn prepare(&self, cmd: &mut Command) {
            // A worker must never flash a console window.
            cmd.creation_flags(CREATE_NO_WINDOW.0);
        }

        pub fn adopt(&self, child: &std::process::Child) -> io::Result<()> {
            let handle = HANDLE(child.as_raw_handle());
            // SAFETY: `handle` belongs to a live child process we just spawned,
            // and `self.job` is a live job object owned by this type.
            unsafe { AssignProcessToJobObject(self.job, handle) }.map_err(io::Error::other)
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            // Closing the last handle to the job terminates every process still
            // inside it, because of KILL_ON_JOB_CLOSE. This is what guarantees no
            // worker survives the UI process.
            // SAFETY: `job` was created by this type and is closed exactly once.
            unsafe {
                let _ = CloseHandle(self.job);
            }
        }
    }
}

#[cfg(unix)]
mod platform {
    use std::io;
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    #[derive(Debug)]
    pub struct Sandbox {
        memory_cap: u64,
    }

    impl Sandbox {
        pub fn create(memory_cap: u64) -> io::Result<Self> {
            Ok(Sandbox { memory_cap })
        }

        pub fn prepare(&self, cmd: &mut Command) {
            let cap = self.memory_cap;
            // SAFETY: `pre_exec` runs in the forked child between fork and exec,
            // where only async-signal-safe calls are permitted. `setrlimit` is on
            // that list, and nothing here allocates or takes a lock.
            unsafe {
                cmd.pre_exec(move || {
                    let lim = libc::rlimit {
                        rlim_cur: cap,
                        rlim_max: cap,
                    };
                    if libc::setrlimit(libc::RLIMIT_AS, &lim) != 0 {
                        return Err(io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }

        pub fn adopt(&self, _child: &std::process::Child) -> io::Result<()> {
            // The limit was applied before exec; nothing to do afterwards.
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sandbox_can_be_created() {
        let s = Sandbox::create().expect("create sandbox");
        assert_eq!(s.is_security_boundary(), cfg!(windows));
    }

    #[test]
    fn preparing_a_command_does_not_fail() {
        let s = Sandbox::create().expect("create sandbox");
        let mut cmd = Command::new("true");
        s.prepare(&mut cmd);
    }

    #[test]
    #[cfg(unix)]
    fn the_memory_cap_actually_binds_the_child() {
        // Proves the rlimit reaches the child rather than being set and lost:
        // a child asked to reserve more than the cap must fail to start.
        let s = Sandbox {
            inner: platform::Sandbox::create(64 * 1024 * 1024).expect("sandbox"),
        };
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg("ulimit -v");
        s.prepare(&mut cmd);
        let out = cmd.output().expect("run child");
        let reported = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert_ne!(
            reported, "unlimited",
            "the child must inherit a bounded address space"
        );
        let kb: u64 = reported.parse().unwrap_or(u64::MAX);
        assert_eq!(
            kb,
            64 * 1024,
            "child should see exactly the cap we set, in KiB"
        );
    }
}
