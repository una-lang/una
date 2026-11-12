//! The verified copy of the Z3 library that `build.rs` links (#1925): the library is copied into
//! a directory of the build's own output that holds that copy alone, the copy's bytes are hashed
//! and checked, and the link directives name only that directory. The linker therefore reads the
//! bytes that were checked, not the file at the given path again after the check, and finds no
//! other library there: a `libstdc++.a`, `libc++.a` or `libc++.tbd` beside the given library is
//! never searched. `build.rs` includes this file as a module and `tests/stage.rs` tests it; it
//! prints nothing and reads no environment variable.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The verified copy: the directory that holds it alone, the `<name>` of its `lib<name>.a`, and
/// the SHA-256 of its bytes.
#[derive(Debug)]
pub struct Staged {
    /// The directory of the copy, which holds nothing else.
    pub dir: PathBuf,
    /// The library's name, `z3shell` for `libz3shell.a`.
    pub name: String,
    /// The SHA-256 of the copy, in lowercase hexadecimal.
    pub sha256: String,
}

impl Staged {
    /// The path of the copy.
    pub fn path(&self) -> PathBuf {
        self.dir.join(format!("lib{}.a", self.name))
    }
}

/// The `<name>` of a static library at `path`, named `lib<name>.a`.
pub fn library_name(path: &Path) -> Result<String, String> {
    let file = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{}: not a file name", path.display()))?;
    let name = file
        .strip_prefix("lib")
        .and_then(|rest| rest.strip_suffix(".a"))
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format!("{file}: the library must be named lib<name>.a"))?;
    Ok(name.to_owned())
}

/// Copies the library at `source` into `dir`, which is emptied first and then holds the copy
/// alone, and hashes the copy. With `want`, the copy's SHA-256 must be that digest; the error
/// names `source`, the file the caller gave. The copy is read-only, and `dir` is listed after the
/// check: anything but the copy in it is an error.
pub fn stage(source: &Path, dir: &Path, want: Option<&str>) -> Result<Staged, String> {
    let name = library_name(source)?;
    // What an earlier build, or anything else, left in the directory goes, so that the
    // linker's search of it finds the copy alone. A symbolic link at `dir` is removed, not
    // followed.
    match fs::remove_dir_all(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    }
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let bytes = fs::read(source).map_err(|e| format!("{}: {e}", source.display()))?;
    let copy = dir.join(format!("lib{name}.a"));
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&copy)
            .map_err(|e| format!("{}: {e}", copy.display()))?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| format!("{}: {e}", copy.display()))?;
    }
    drop(bytes);
    // The digest is of the copy as it is on disk, the bytes the linker reads.
    let copied = fs::read(&copy).map_err(|e| format!("{}: {e}", copy.display()))?;
    let sha256 = format!("{:x}", Sha256::digest(&copied));
    if let Some(want) = want
        && want != sha256
    {
        return Err(format!(
            "SHA-256 mismatch for {}: UNA_EMBED_Z3_SHA256 is {want}, the file is {sha256}",
            source.display()
        ));
    }
    let mut permissions = fs::metadata(&copy)
        .map_err(|e| format!("{}: {e}", copy.display()))?
        .permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&copy, permissions).map_err(|e| format!("{}: {e}", copy.display()))?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        entries.push(entry.file_name());
    }
    if entries != [copy.file_name().unwrap_or_default().to_owned()] {
        return Err(format!(
            "{}: the directory of the verified Z3 library holds more than the library: {entries:?}",
            dir.display()
        ));
    }
    Ok(Staged {
        dir: dir.to_path_buf(),
        name,
        sha256,
    })
}

/// The cargo directives that link the verified copy and the C++ runtime the front end needs,
/// for the target OS `target_os` (`CARGO_CFG_TARGET_OS`): the system's `libc++` on macOS,
/// dynamically; on Linux `libstdc++`, statically, so that `una` asks the dynamic loader for no
/// C++ library (ADR 0124 rule 1, `scripts/load-commands-una.txt`, #1956). The driver flag
/// `-static-libstdc++` of `scripts/release.sh` does not reach an explicit `-lstdc++`, so the kind
/// is given here: `static`, without `bundle`, so that rustc does not copy the archive into this
/// crate's rlib and the final link takes the toolchain's `libstdc++.a` from the C compiler's own
/// search path. The one search directory is the copy's, which holds nothing else.
pub fn directives(staged: &Staged, target_os: Option<&str>) -> Result<Vec<String>, String> {
    let runtime = match target_os {
        Some("macos") => "cargo:rustc-link-lib=c++",
        Some("linux") => "cargo:rustc-link-lib=static:-bundle=stdc++",
        other => {
            return Err(format!(
                "una-z3 links on macOS and Linux only, not {other:?}"
            ));
        }
    };
    Ok(vec![
        format!("cargo:rustc-link-search=native={}", staged.dir.display()),
        format!("cargo:rustc-link-lib=static={}", staged.name),
        runtime.to_owned(),
    ])
}
