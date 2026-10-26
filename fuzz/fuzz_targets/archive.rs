//! The `archive` fuzz target (#997): the reader of a package archive (ADR 0122 rule 29), on any
//! bytes. It must never panic, and an archive it reads is canonical: its files, taken from the
//! ranges it gives, make the same archive again, with the same content digest.
#![no_main]

use libfuzzer_sys::fuzz_target;
use una_project::content::{self, File};

fuzz_target!(|bytes: &[u8]| {
    if let Ok(read) = content::read_archive(bytes) {
        let mut files: Vec<File> = read
            .files
            .iter()
            .map(|(path, range)| File {
                path: path.clone(),
                bytes: bytes.get(range.clone()).map(<[u8]>::to_vec).unwrap_or_default(),
            })
            .collect();
        assert_eq!(content::digest_of(&mut files).as_ref(), Ok(&read.digest));
        assert_eq!(content::archive(&mut files).as_deref(), Ok(bytes));
    }
});
