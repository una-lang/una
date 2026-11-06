//! The processor time of this process's children, for the parent that measures their work
//! rather than the wall clock (#1656): a child's user and system time do not grow when other
//! processes share the machine, where its elapsed time does. The standard library has no
//! interface to another process's processor time.
#![allow(unsafe_code)]

use std::io;
use std::time::Duration;

/// The user and system processor time of every child of this process that has ended and been
/// waited for, their own waited-for descendants included (`getrusage(RUSAGE_CHILDREN)`). The
/// time of one child is the growth of this reading across its run and wait.
///
/// # Errors
///
/// The error of `getrusage`, which for this flavor and a valid pointer is none in practice.
pub fn children_cpu_time() -> io::Result<Duration> {
    // SAFETY: zero is a valid, if empty, `rusage`, which the call fills in whole.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: `usage` is a valid `rusage` that the call writes through the pointer; no other
    // memory is read or written.
    let read = unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, &raw mut usage) };
    if read == 0 {
        Ok(duration(usage.ru_utime) + duration(usage.ru_stime))
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `time` as a duration; the kernel never reports a negative part, which would count as zero.
fn duration(time: libc::timeval) -> Duration {
    let seconds = u64::try_from(time.tv_sec).unwrap_or(0);
    let micros = u64::try_from(time.tv_usec).unwrap_or(0);
    Duration::from_secs(seconds) + Duration::from_micros(micros)
}

#[cfg(test)]
mod tests {
    // Tests may unwrap freely (compiler/conventions.md §1).
    #![allow(clippy::unwrap_used)]

    use super::children_cpu_time;

    #[test]
    fn the_reading_never_goes_back_across_a_waited_child() {
        let before = children_cpu_time().unwrap();
        let status = std::process::Command::new("sh")
            .args(["-c", "i=0; while [ $i -lt 20000 ]; do i=$((i+1)); done"])
            .status()
            .unwrap();
        assert!(status.success());
        let after = children_cpu_time().unwrap();
        assert!(after >= before, "{before:?} then {after:?}");
    }
}
