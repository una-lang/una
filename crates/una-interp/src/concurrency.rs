//! The settings of tasks that both engines read from the command line (ADR 0117 rules 2 and 3):
//! `--workers`, `--task-depth`, `--io-threads` and `--file-threads`, and the serve threads' `--serve-threads` (ADR 0123 rule 20), with their ranges and
//! defaults. A value that is not a number, or is out of its range, is a usage error, exit code
//! 2 (ADR 0117, Diagnostics).

/// The most `--workers` allows.
pub const MAX_WORKERS: usize = 1024;
/// The most `--io-threads` allows.
pub const MAX_IO_THREADS: usize = 4096;
/// The default of `--io-threads`.
pub const DEFAULT_IO_THREADS: usize = 64;
/// The most `--file-threads` allows.
pub const MAX_FILE_THREADS: usize = 256;
/// The default of `--file-threads`.
pub const DEFAULT_FILE_THREADS: usize = 4;
/// The most `--serve-threads` allows (ADR 0123 rule 20).
pub const MAX_SERVE_THREADS: usize = 256;
/// The default of `--task-depth`, lowered to `--max-depth` when that is smaller.
pub const DEFAULT_TASK_DEPTH: usize = 1000;

/// The settings of a run's tasks, checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Concurrency {
    /// `--workers`: the threads that run tasks, from 1 to [`MAX_WORKERS`].
    pub workers: usize,
    /// `--task-depth`: the depth limit of the tasks of `first of` and `start`, from 1 to
    /// `--max-depth`.
    pub task_depth: usize,
    /// `--io-threads`: the helpers of the `Http` pool, from 1 to [`MAX_IO_THREADS`].
    pub io_threads: usize,
    /// `--file-threads`: the helpers of the `Files` and `Console` pool, from 1 to
    /// [`MAX_FILE_THREADS`].
    pub file_threads: usize,
}

impl Concurrency {
    /// The defaults for a run whose maximum depth is `max_depth`: a worker per core of the
    /// machine, at most [`MAX_WORKERS`].
    pub fn defaults(max_depth: usize) -> Concurrency {
        let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        Concurrency {
            workers: cores.clamp(1, MAX_WORKERS),
            task_depth: DEFAULT_TASK_DEPTH.min(max_depth).max(1),
            io_threads: DEFAULT_IO_THREADS,
            file_threads: DEFAULT_FILE_THREADS,
        }
    }

    /// The settings from the values given on the command line, each `None` when its flag is
    /// absent, or the message of the usage error. `--task-depth` is checked against
    /// `max_depth` once every flag is read, so the order of the flags does not matter.
    pub fn from_flags(
        max_depth: usize,
        workers: Option<usize>,
        task_depth: Option<usize>,
        io_threads: Option<usize>,
        file_threads: Option<usize>,
    ) -> Result<Concurrency, String> {
        let mut settings = Concurrency::defaults(max_depth);
        if let Some(n) = workers {
            settings.workers = within("--workers", n, MAX_WORKERS)?;
        }
        if let Some(n) = task_depth {
            if n > max_depth {
                return Err(format!(
                    "`--task-depth {n}` is deeper than `--max-depth {max_depth}`; give at most {max_depth}"
                ));
            }
            settings.task_depth = within("--task-depth", n, max_depth.max(1))?;
        }
        if let Some(n) = io_threads {
            settings.io_threads = within("--io-threads", n, MAX_IO_THREADS)?;
        }
        if let Some(n) = file_threads {
            settings.file_threads = within("--file-threads", n, MAX_FILE_THREADS)?;
        }
        Ok(settings)
    }
}

/// The serve threads of a run that serves (ADR 0123 rule 20): `--serve-threads` from 1 to
/// [`MAX_SERVE_THREADS`], or by default the number of the machine's cores within that range. A
/// run that does not serve accepts the flag and uses none.
pub fn serve_threads(given: Option<usize>) -> Result<usize, String> {
    match given {
        Some(n) => within("--serve-threads", n, MAX_SERVE_THREADS),
        None => {
            let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
            Ok(cores.clamp(1, MAX_SERVE_THREADS))
        }
    }
}

impl Default for Concurrency {
    /// The defaults for the default maximum depth.
    fn default() -> Concurrency {
        Concurrency::defaults(crate::DEFAULT_MAX_DEPTH)
    }
}

/// The most started tasks a run holds at once on this system (ADR 0117 rule 2): on Linux a
/// quarter of `vm.max_map_count`, read when called, since each task's stack takes two memory
/// mappings; `None` elsewhere, and when the setting cannot be read.
pub fn task_cap() -> Option<usize> {
    if cfg!(target_os = "linux") {
        let text = std::fs::read_to_string("/proc/sys/vm/max_map_count").ok()?;
        let count: usize = text.trim().parse().ok()?;
        Some((count / 4).max(1))
    } else {
        None
    }
}

/// The bound of a `tasks` statement written without one (ADR 0117 rule 7): `max(64, M)`, with
/// `M` the memory budget in MiB, within `cap`, the cap of [`task_cap`].
pub fn default_bound(max_memory: usize, cap: Option<usize>) -> usize {
    let bound = max_memory.max(64);
    cap.map_or(bound, |cap| bound.min(cap))
}

/// How many `tasks` statements, or `first of` blocks, a task has run, for the paths of the
/// tasks they start (`spec/27 §6` rule 5, `§7` rule 5): those of public code and those of quiet
/// code apart, so that whether quiet code ran a block, which may depend on data above `public`,
/// never moves the number of a later public one (S-39, `spec/26 §6` rule 4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlockCounts {
    public: usize,
    quiet: usize,
}

impl BlockCounts {
    /// Counts one more block, in quiet code or not, and returns its number there, from 1.
    pub fn next(&mut self, quiet: bool) -> usize {
        let count = if quiet {
            &mut self.quiet
        } else {
            &mut self.public
        };
        *count = count.saturating_add(1);
        *count
    }
}

/// The part of a task path that names the `count`-th block of kind `kind` (`s` for `tasks`,
/// `f` for `first of`) of its task: `<kind><count>`, or `q<kind><count>` in quiet code.
pub fn block_name(kind: char, quiet: bool, count: usize) -> String {
    if quiet {
        format!("q{kind}{count}")
    } else {
        format!("{kind}{count}")
    }
}

/// The id of an instance of the handler `name` that quiet code installs, as both engines name
/// it: `with:<name>#q`, with no number (S-41, `spec/26 §6` rule 4). Quiet code counts none of
/// its installations, so how many it ran, which may depend on data above `public`, shows in no
/// id: neither in the `n` of a later public installation nor in a later quiet one's. A join
/// leaves the id as it is, since it names no task path.
pub fn quiet_install_id(name: &str) -> String {
    format!("with:{name}#q")
}

/// The one line on stderr of a started task that ran too deep and ended alone (`spec/27 §7`,
/// ADR 0117 rule 7), as both engines write it: U6003, the task's path, why, and the position
/// `file:line:column` of the call, when it is known.
pub fn ended_alone(path: &str, message: &str, at: Option<(&str, u32, u32)>) -> String {
    let at = at.map_or_else(String::new, |(file, line, col)| {
        format!(" at {file}:{line}:{col}")
    });
    format!("[U6003] Error: the started task {path} ended alone: {message}{at}")
}

/// `n` when it is from 1 to `most`, or the message of the usage error for `flag`.
fn within(flag: &str, n: usize, most: usize) -> Result<usize, String> {
    if n == 0 {
        return Err(format!("`{flag} 0` allows nothing; give at least 1"));
    }
    if n > most {
        return Err(format!(
            "`{flag} {n}` is above its limit; give at most {most}"
        ));
    }
    Ok(n)
}

/// A value of one of these flags: a whole number, or the message of the usage error for
/// `flag`.
pub fn number(flag: &str, text: &str) -> Result<usize, String> {
    text.parse()
        .map_err(|_| format!("`{flag} {text}` is not a number"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_blocks_are_counted_apart_from_public_ones() {
        // S-39: quiet code never moves the number of a later public block.
        let mut counts = BlockCounts::default();
        assert_eq!(counts.next(false), 1);
        assert_eq!(counts.next(true), 1);
        assert_eq!(counts.next(true), 2);
        assert_eq!(counts.next(false), 2);
        assert_eq!(block_name('s', false, 2), "s2");
        assert_eq!(block_name('f', true, 2), "qf2");
    }

    #[test]
    fn flags_take_their_ranges_and_defaults() {
        let defaults = Concurrency::from_flags(10_000, None, None, None, None).unwrap();
        assert!((1..=MAX_WORKERS).contains(&defaults.workers));
        assert_eq!(
            (
                defaults.task_depth,
                defaults.io_threads,
                defaults.file_threads
            ),
            (1000, 64, 4)
        );
        assert_eq!(Concurrency::defaults(50).task_depth, 50);
        let given = Concurrency::from_flags(10_000, Some(1), Some(7), Some(4096), Some(256));
        assert_eq!(
            given,
            Ok(Concurrency {
                workers: 1,
                task_depth: 7,
                io_threads: 4096,
                file_threads: 256
            })
        );
        for wrong in [
            Concurrency::from_flags(10_000, Some(0), None, None, None),
            Concurrency::from_flags(10_000, Some(1025), None, None, None),
            Concurrency::from_flags(100, None, Some(101), None, None),
            Concurrency::from_flags(100, None, Some(0), None, None),
            Concurrency::from_flags(100, None, None, Some(4097), None),
            Concurrency::from_flags(100, None, None, None, Some(257)),
        ] {
            assert!(wrong.is_err());
        }
        assert!(number("--workers", "many").is_err());
        assert_eq!(number("--workers", "3"), Ok(3));
    }

    #[test]
    fn serve_threads_take_their_range_and_default_to_the_cores() {
        assert!((1..=MAX_SERVE_THREADS).contains(&serve_threads(None).unwrap()));
        assert_eq!(serve_threads(Some(1)), Ok(1));
        assert_eq!(serve_threads(Some(256)), Ok(256));
        assert!(serve_threads(Some(0)).is_err());
        assert!(serve_threads(Some(257)).is_err());
    }

    #[test]
    fn the_default_bound_of_tasks_is_the_budget_in_mib_within_the_cap() {
        assert_eq!(default_bound(4096, None), 4096);
        assert_eq!(default_bound(16, None), 64);
        assert_eq!(default_bound(4096, Some(1000)), 1000);
        assert_eq!(default_bound(16, Some(10)), 10);
        if cfg!(target_os = "linux") {
            assert!(task_cap().is_some_and(|cap| cap >= 1));
        } else {
            assert_eq!(task_cap(), None);
        }
    }
}
