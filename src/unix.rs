//! This module defines the unix thread control.
//!
//! The crate's prelude doesn't have much control over
//! the unix threads, and this module provides
//! better control over those.

use std::convert::TryFrom;

use crate::{Error, ThreadPriority, ThreadPriorityValue};

/// An alias type for a thread id.
pub type ThreadId = libc::pthread_t;

/// Proxy structure to maintain compatibility between glibc and musl
pub struct ScheduleParams {
    /// Copy of `sched_priority` from `libc::sched_param`
    pub sched_priority: libc::c_int,
}

/// Copy of the Linux kernel's sched_attr type
#[repr(C)]
#[derive(Debug, Default)]
#[cfg(target_os = "linux")]
pub struct SchedAttr {
    size: u32,
    sched_policy: u32,
    sched_flags: u64,

    /// for SCHED_NORMAL and SCHED_BATCH
    sched_nice: i32,
    /// for SCHED_FIFO, SCHED_RR
    sched_priority: u32,

    /// for SCHED_DEADLINE
    sched_runtime: u64,
    /// for SCHED_DEADLINE
    sched_deadline: u64,
    /// for SCHED_DEADLINE
    sched_period: u64,

    /// Utilization hint
    sched_util_min: u32,
    /// Utilization hint
    sched_util_max: u32,
}

impl ScheduleParams {
    #[cfg(not(target_env = "musl"))]
    fn into_posix(self) -> libc::sched_param {
        libc::sched_param {
            sched_priority: self.sched_priority,
        }
    }

    #[cfg(target_env = "musl")]
    fn into_posix(self) -> libc::sched_param {
        use libc::timespec as TimeSpec;

        libc::sched_param {
            sched_priority: self.sched_priority,
            sched_ss_low_priority: 0,
            sched_ss_repl_period: TimeSpec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            sched_ss_init_budget: TimeSpec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            sched_ss_max_repl: 0,
        }
    }

    fn from_posix(sched_param: libc::sched_param) -> Self {
        ScheduleParams {
            sched_priority: sched_param.sched_priority,
        }
    }
}

/// The following "real-time" policies are also supported, for special time-critical applications
/// that need precise control over the way in which runnable processes are selected for execution
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum RealtimeThreadSchedulePolicy {
    /// A first-in, first-out policy
    Fifo,
    /// A round-robin policy
    RoundRobin,
    /// A deadline policy. Note, due to Linux expecting a pid_t and not a pthread_t, the given
    /// [ThreadId](struct.ThreadId) will be interpreted as a pid_t. This policy is NOT
    /// POSIX-compatible, so we only include it for linux targets.
    #[cfg(target_os = "linux")]
    Deadline,
}
impl RealtimeThreadSchedulePolicy {
    fn to_posix(self) -> libc::c_int {
        match self {
            RealtimeThreadSchedulePolicy::Fifo => 1,
            RealtimeThreadSchedulePolicy::RoundRobin => 2,
            #[cfg(target_os = "linux")]
            RealtimeThreadSchedulePolicy::Deadline => 6,
        }
    }
}

/// Flags for controlling Deadline scheduling behavior.
#[derive(Debug, Clone, Copy, Ord, PartialEq, Eq, PartialOrd, Hash)]
pub enum DeadlineFlags {
    /// Children created by fork will not inhered privileged scheduling policies.
    ResetOnFork = 0x01,
    /// The thread may reclaim bandwidth that is unused by another realtime thread.
    Reclaim = 0x02,
    /// Request to be send SIGXCPU when this thread overruns its deadline.
    DeadlineOverrun = 0x04,
}

/// Normal (usual) schedule policies
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum NormalThreadSchedulePolicy {
    /// For running very low priority background jobs
    Idle,
    /// For "batch" style execution of processes
    Batch,
    /// The standard round-robin time-sharing policy
    Other,
    /// The standard round-robin time-sharing policy
    Normal,
}
impl NormalThreadSchedulePolicy {
    fn to_posix(self) -> libc::c_int {
        match self {
            NormalThreadSchedulePolicy::Idle => 5,
            NormalThreadSchedulePolicy::Batch => 3,
            NormalThreadSchedulePolicy::Other | NormalThreadSchedulePolicy::Normal => 0,
        }
    }
}

/// Thread schedule policy definition
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ThreadSchedulePolicy {
    /// Normal thread schedule policies
    Normal(NormalThreadSchedulePolicy),
    /// Realtime thread schedule policies
    Realtime(RealtimeThreadSchedulePolicy),
}
impl ThreadSchedulePolicy {
    fn to_posix(self) -> libc::c_int {
        match self {
            ThreadSchedulePolicy::Normal(p) => p.to_posix(),
            ThreadSchedulePolicy::Realtime(p) => p.to_posix(),
        }
    }

    fn from_posix(policy: libc::c_int) -> Result<ThreadSchedulePolicy, Error> {
        match policy {
            0 => Ok(ThreadSchedulePolicy::Normal(
                NormalThreadSchedulePolicy::Normal,
            )),
            3 => Ok(ThreadSchedulePolicy::Normal(
                NormalThreadSchedulePolicy::Batch,
            )),
            5 => Ok(ThreadSchedulePolicy::Normal(
                NormalThreadSchedulePolicy::Idle,
            )),
            1 => Ok(ThreadSchedulePolicy::Realtime(
                RealtimeThreadSchedulePolicy::Fifo,
            )),
            2 => Ok(ThreadSchedulePolicy::Realtime(
                RealtimeThreadSchedulePolicy::RoundRobin,
            )),
            #[cfg(target_os = "linux")]
            6 => Ok(ThreadSchedulePolicy::Realtime(
                RealtimeThreadSchedulePolicy::Deadline,
            )),
            _ => Err(Error::Ffi("Can't parse schedule policy from posix")),
        }
    }
}

impl ThreadPriority {
    /// POSIX value can not be known without knowing the scheduling policy
    /// <https://linux.die.net/man/2/sched_get_priority_max>
    pub fn to_posix(self, policy: ThreadSchedulePolicy) -> Result<libc::c_int, Error> {
        let ret = match self {
            ThreadPriority::Min => match policy {
                // SCHED_DEADLINE doesn't really have a notion of priority, this is an error
                #[cfg(target_os = "linux")]
                ThreadSchedulePolicy::Realtime(RealtimeThreadSchedulePolicy::Deadline) => Err(
                    Error::Priority("Deadline scheduling must use deadline priority."),
                ),
                ThreadSchedulePolicy::Realtime(_) => Ok(1),
                _ => Ok(0),
            },
            ThreadPriority::Crossplatform(ThreadPriorityValue(p)) => match policy {
                // SCHED_DEADLINE doesn't really have a notion of priority, this is an error
                #[cfg(target_os = "linux")]
                ThreadSchedulePolicy::Realtime(RealtimeThreadSchedulePolicy::Deadline) => Err(
                    Error::Priority("Deadline scheduling must use deadline priority."),
                ),
                ThreadSchedulePolicy::Realtime(_) if (p == 0 || p > 99) => {
                    Err(Error::Priority("The value is out of range [0; 99]"))
                }
                ThreadSchedulePolicy::Normal(_) if p != 0 => Err(Error::Priority(
                    "The value can be only 0 for normal scheduling policy",
                )),
                _ => Ok(p as u32),
            },
            // TODO avoid code duplication.
            ThreadPriority::Os(crate::ThreadPriorityOsValue(p)) => match policy {
                // SCHED_DEADLINE doesn't really have a notion of priority, this is an error
                #[cfg(target_os = "linux")]
                ThreadSchedulePolicy::Realtime(RealtimeThreadSchedulePolicy::Deadline) => Err(
                    Error::Priority("Deadline scheduling must use deadline priority."),
                ),
                ThreadSchedulePolicy::Realtime(_) if (p == 0 || p > 99) => {
                    Err(Error::Priority("The value is out of range [0; 99]"))
                }
                ThreadSchedulePolicy::Normal(_) if p != 0 => Err(Error::Priority(
                    "The value can be only 0 for normal scheduling policy",
                )),
                _ => Ok(p),
            },
            ThreadPriority::Max => match policy {
                // SCHED_DEADLINE doesn't really have a notion of priority, this is an error
                #[cfg(target_os = "linux")]
                ThreadSchedulePolicy::Realtime(RealtimeThreadSchedulePolicy::Deadline) => Err(
                    Error::Priority("Deadline scheduling must use deadline priority."),
                ),
                ThreadSchedulePolicy::Realtime(_) => Ok(99),
                _ => Ok(0),
            },
            #[cfg(target_os = "linux")]
            ThreadPriority::Deadline(_, _, _, _) => Err(Error::Priority(
                "Deadline is non-POSIX and cannot be converted.",
            )),
        };
        ret.map(|p| p as libc::c_int)
    }

    /// Gets priority value from POSIX value.
    /// In order to interpret it correctly, you should also take scheduling policy
    /// into account.
    pub fn from_posix(params: ScheduleParams) -> ThreadPriority {
        ThreadPriority::Crossplatform(ThreadPriorityValue(params.sched_priority as u8))
    }
}

/// Sets thread's priority and schedule policy
///
/// * May require privileges
///
/// # Usage
///
/// Setting thread priority to minimum with normal schedule policy:
///
/// ```rust
/// use thread_priority::*;
///
/// let thread_id = thread_native_id();
/// assert!(set_thread_priority_and_policy(thread_id,
///                                        ThreadPriority::Min,
///                                        ThreadSchedulePolicy::Normal(NormalThreadSchedulePolicy::Normal)).is_ok());
/// ```
pub fn set_thread_priority_and_policy(
    native: ThreadId,
    priority: ThreadPriority,
    policy: ThreadSchedulePolicy,
) -> Result<(), Error> {
    let params = ScheduleParams {
        sched_priority: match policy {
            ThreadSchedulePolicy::Realtime(RealtimeThreadSchedulePolicy::Deadline) => 0,
            _ => priority.to_posix(policy)?,
        },
    };
    set_thread_schedule_policy(native, policy, params, priority)
}

/// Set current thread's priority.
pub fn set_current_thread_priority(priority: ThreadPriority) -> Result<(), Error> {
    let thread_id = thread_native_id();
    let policy = ThreadSchedulePolicy::Normal(NormalThreadSchedulePolicy::Normal);
    set_thread_priority_and_policy(thread_id, priority, policy)
}

/// Returns policy parameters (schedule policy and other schedule parameters) for current process
///
/// # Usage
///
/// ```rust
/// use thread_priority::*;
///
/// assert!(thread_schedule_policy().is_ok());
/// ```
pub fn thread_schedule_policy() -> Result<ThreadSchedulePolicy, Error> {
    unsafe { ThreadSchedulePolicy::from_posix(libc::sched_getscheduler(libc::getpid())) }
}

/// Sets thread schedule policy.
///
/// * May require privileges
/// * Deadline policy requires a tid, not a pthread_t, so invoking this while using a deadline
/// policy will interpret the given [ThreadId](struct.ThreadId) as a pid_t (thread tid).
///
/// # Usage
/// ```rust,no_run
/// use thread_priority::*;
///
/// let thread_id = thread_native_id();
/// let policy = ThreadSchedulePolicy::Realtime(RealtimeThreadSchedulePolicy::Fifo);
/// let params = ScheduleParams { sched_priority: 3 as libc::c_int };
/// let priority = ThreadPriority::Min;
/// assert!(set_thread_schedule_policy(thread_id, policy, params, priority).is_ok());
/// ```
pub fn set_thread_schedule_policy(
    native: ThreadId,
    policy: ThreadSchedulePolicy,
    params: ScheduleParams,
    priority: ThreadPriority,
) -> Result<(), Error> {
    let params = params.into_posix();
    unsafe {
        let ret = match policy {
            // SCHED_DEADLINE policy requires its own syscall
            #[cfg(target_os = "linux")]
            ThreadSchedulePolicy::Realtime(RealtimeThreadSchedulePolicy::Deadline) => {
                let (runtime, deadline, period, flags) = match priority {
                    ThreadPriority::Deadline(r, d, p, f) => (r, d, p, f),
                    _ => {
                        return Err(Error::Priority(
                            "Deadline policy given without deadline priority.",
                        ))
                    }
                };
                let tid = native as libc::pid_t;
                let sched_attr = SchedAttr {
                    size: std::mem::size_of::<SchedAttr>() as u32,
                    sched_policy: policy.to_posix() as u32,

                    sched_runtime: runtime as u64,
                    sched_deadline: deadline as u64,
                    sched_period: period as u64,

                    ..Default::default()
                };
                libc::syscall(
                    libc::SYS_sched_setattr,
                    tid,
                    &sched_attr as *const _,
                    // we are not setting SCHED_FLAG_RECLAIM nor SCHED_FLAG_DL_OVERRUN
                    match flags {
                        None => 0,
                        Some(flags) => flags as i32,
                    },
                ) as i32
            }
            _ => libc::pthread_setschedparam(
                native,
                policy.to_posix(),
                &params as *const libc::sched_param,
            ),
        };
        match ret {
            0 => Ok(()),
            e => Err(Error::OS(e)),
        }
    }
}

/// Returns policy parameters (schedule policy and other schedule parameters)
///
/// # Usage
///
/// ```rust
/// use thread_priority::*;
///
/// let thread_id = thread_native_id();
/// assert!(thread_schedule_policy_param(thread_id).is_ok());
/// ```
pub fn thread_schedule_policy_param(
    native: ThreadId,
) -> Result<(ThreadSchedulePolicy, ScheduleParams), Error> {
    unsafe {
        let mut policy = 0i32;
        let mut params = ScheduleParams { sched_priority: 0 }.into_posix();

        let ret = libc::pthread_getschedparam(
            native,
            &mut policy as *mut libc::c_int,
            &mut params as *mut libc::sched_param,
        );
        match ret {
            0 => Ok((
                ThreadSchedulePolicy::from_posix(policy)?,
                ScheduleParams::from_posix(params),
            )),
            e => Err(Error::OS(e)),
        }
    }
}

/// Get current thread's priority value.
pub fn thread_priority() -> Result<ThreadPriority, Error> {
    Ok(ThreadPriority::from_posix(
        thread_schedule_policy_param(thread_native_id())?.1,
    ))
}

/// A helper trait for other threads to implement to be able to call methods
/// on threads themselves.
///
/// ```rust
/// use thread_priority::*;
///
/// assert!(std::thread::current().get_priority().is_ok());
///
/// let join_handle = std::thread::spawn(|| println!("Hello world!"));
/// assert!(join_handle.thread().get_priority().is_ok());
///
/// join_handle.join();
/// ```
pub trait ThreadExt {
    /// Gets the current thread's priority.
    /// For more info read [`thread_priority`].
    ///
    /// ```rust
    /// use thread_priority::*;
    ///
    /// assert!(std::thread::current().get_priority().is_ok());
    /// ```
    fn get_priority(&self) -> Result<ThreadPriority, Error> {
        thread_priority()
    }

    /// Sets the current thread's priority.
    /// For more info see [`ThreadPriority::set_for_current`].
    ///
    /// ```rust
    /// use thread_priority::*;
    ///
    /// assert!(std::thread::current().set_priority(ThreadPriority::Min).is_ok());
    /// ```
    fn set_priority(&self, priority: ThreadPriority) -> Result<(), Error> {
        priority.set_for_current()
    }

    /// Gets the current thread's schedule policy.
    /// For more info read [`thread_schedule_policy`].
    fn get_schedule_policy(&self) -> Result<ThreadSchedulePolicy, Error> {
        thread_schedule_policy()
    }

    /// Returns current thread's schedule policy and parameters.
    /// For more info read [`thread_schedule_policy_param`].
    fn get_schedule_policy_param(&self) -> Result<(ThreadSchedulePolicy, ScheduleParams), Error> {
        thread_schedule_policy_param(thread_native_id())
    }

    /// Sets current thread's schedule policy.
    /// For more info read [`set_thread_schedule_policy`].
    fn set_schedule_policy(
        &self,
        policy: ThreadSchedulePolicy,
        priority: ThreadPriority,
    ) -> Result<(), Error> {
        let params = ScheduleParams {
            sched_priority: match policy {
                ThreadSchedulePolicy::Realtime(RealtimeThreadSchedulePolicy::Deadline) => 0,
                _ => priority.to_posix(policy)?,
            },
        };
        set_thread_schedule_policy(thread_native_id(), policy, params, priority)
    }

    /// Returns native unix thread id.
    /// For more info read [`thread_native_id`].
    ///
    /// ```rust
    /// use thread_priority::*;
    ///
    /// assert!(std::thread::current().get_native_id() > 0);
    fn get_native_id(&self) -> ThreadId {
        thread_native_id()
    }
}

/// Auto-implementation of this trait for the [`std::thread::Thread`].
impl ThreadExt for std::thread::Thread {}

/// Returns current thread id, which is the current OS's native handle.
/// It may or may not be equal or even related to rust's thread id,
/// there is absolutely no guarantee for that.
///
/// # Usage
///
/// ```rust
/// use thread_priority::thread_native_id;
///
/// assert!(thread_native_id() > 0);
/// ```
pub fn thread_native_id() -> ThreadId {
    unsafe { libc::pthread_self() }
}

impl TryFrom<u8> for ThreadPriority {
    type Error = &'static str;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if let 0..=100 = value {
            Ok(ThreadPriority::Crossplatform(ThreadPriorityValue(value)))
        } else {
            Err("The thread priority value must be in range of [0; 100].")
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::unix::*;

    #[test]
    fn thread_schedule_policy_param_test() {
        let thread_id = thread_native_id();

        assert!(thread_schedule_policy_param(thread_id).is_ok());
    }

    #[test]
    fn set_thread_priority_test() {
        let thread_id = thread_native_id();

        assert!(set_thread_priority_and_policy(
            thread_id,
            ThreadPriority::Min,
            ThreadSchedulePolicy::Normal(NormalThreadSchedulePolicy::Normal)
        )
        .is_ok());
        assert!(set_thread_priority_and_policy(
            thread_id,
            ThreadPriority::Max,
            ThreadSchedulePolicy::Normal(NormalThreadSchedulePolicy::Normal)
        )
        .is_ok());
        assert!(set_thread_priority_and_policy(
            thread_id,
            ThreadPriority::Crossplatform(ThreadPriorityValue(0)),
            ThreadSchedulePolicy::Normal(NormalThreadSchedulePolicy::Normal)
        )
        .is_ok());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn set_deadline_policy() {
        // allow the identity operation for clarity
        #![allow(clippy::identity_op)]

        assert!(set_thread_priority_and_policy(
            0, // current thread
            ThreadPriority::Deadline(
                1 * 10_u64.pow(6),
                10 * 10_u64.pow(6),
                100 * 10_u64.pow(6),
                None
            ),
            ThreadSchedulePolicy::Realtime(RealtimeThreadSchedulePolicy::Deadline)
        )
        .is_ok());

        // now we check the return values
        unsafe {
            let mut sched_attr = SchedAttr::default();
            let ret = libc::syscall(
                libc::SYS_sched_getattr,
                0, // current thread
                &mut sched_attr as *mut _,
                std::mem::size_of::<SchedAttr>() as u32,
                0, // flags must be 0
            );

            assert!(ret >= 0);
            assert_eq!(
                sched_attr.sched_policy,
                RealtimeThreadSchedulePolicy::Deadline.to_posix() as u32
            );
            assert_eq!(sched_attr.sched_runtime, 1 * 10_u64.pow(6));
            assert_eq!(sched_attr.sched_deadline, 10 * 10_u64.pow(6));
            assert_eq!(sched_attr.sched_period, 100 * 10_u64.pow(6));
        }
    }
}
