//! The registry client as `una ita` uses it (ADR 0122 rules 25, 26, 34 to 40, `spec/30 §14`):
//! where each source of the root is read from, opening it verified, and the lines a lock
//! writes for a release.
//!
//! **Where a source is read from.** A `--mirror <source>=<location>` names a mirror directory,
//! or a `file://` URL of one, for that source; with none, the source is read at the `https://`
//! URL `ita.una`'s `sources` gives it. This release has no https transport: a source with no
//! mirror is refused with U7013, which says so, and is never skipped. A location with any other
//! scheme, `http://` included, is refused when it is read (U7013), so plain HTTP is never used.
//! A mirror is verified as the source itself: the root from the pin of `sources`, the
//! checkpoint and its witnesses, every proof and signature, and every archive's digest.
//!
//! `itaty`, whose root metadata is to be built into `una`, is not served by this release: a need
//! of it stays U7013. No program is started and no connection is opened.

use std::cell::RefCell;
use std::collections::BTreeMap;

use una_diag::Code;
use una_project::ProjectError;
use una_project::client::{LoggedRelease, Point, Registry};
use una_project::ita::ITATY;
use una_project::itaoca::Itaoca;
use una_project::transport::{self, Location, Transport};
use una_syntax::ita;

/// The values of `--mirror <source>=<location>`, by source. A value that is not of that form,
/// a source named twice, or a location that is not a directory, a `file://` URL or an
/// `https://` URL, is refused with U7013 at the start of `ita.una`: plain HTTP is never used.
pub(crate) fn mirrors(values: &[String]) -> Result<BTreeMap<String, Location>, ProjectError> {
    let refuse = |message: String| ProjectError {
        code: Code::U7013,
        range: 0..0,
        message,
    };
    let mut out = BTreeMap::new();
    for value in values {
        let Some((source, location)) = value.split_once('=') else {
            return Err(refuse(format!(
                "`--mirror {value}` is not `<source>=<location>`"
            )));
        };
        if una_project::vendor::check_source(source).is_err() || source == ITATY {
            return Err(refuse(format!(
                "`{source}` in `--mirror` is not a source this release reads"
            )));
        }
        let location = Location::parse(location)
            .map_err(|why| refuse(format!("the mirror of `{source}` is refused: {why}")))?;
        if out.insert(source.to_owned(), location).is_some() {
            return Err(refuse(format!("`--mirror` names `{source}` twice")));
        }
    }
    Ok(out)
}

/// The transport of `source`: its mirror when one is named, else the URL its `sources` entry
/// gives. A source read over https is refused here with the reason, since this release has no
/// https transport: the refusal says how to read it from a mirror instead.
pub(crate) fn transport_of(
    source: &ita::Source,
    mirrors: &BTreeMap<String, Location>,
) -> Result<Box<dyn Transport>, String> {
    let name = &source.name.value;
    let location = match mirrors.get(name) {
        Some(location) => location.clone(),
        None => Location::parse(&source.url.value)
            .map_err(|why| format!("the source `{name}` is refused: {why}"))?,
    };
    match location {
        Location::Https(url) => Err(format!(
            "the source `{name}` is at `{url}`, and this release has no https transport; read it from a mirror directory with `--mirror {name}=<directory>`"
        )),
        Location::Directory(_) => Ok(transport::open(&location)),
    }
}

/// The digest of the root metadata that `source` pins.
pub(crate) fn pin_of(source: &ita::Source) -> Result<&str, String> {
    source
        .root
        .as_ref()
        .map(|root| root.value.as_str())
        .ok_or_else(|| format!("the source `{}` pins no root", source.name.value))
}

/// The source `ita.una` declares under `name`.
pub(crate) fn declared<'m>(read: &'m ita::Manifest, name: &str) -> Option<&'m ita::Source> {
    read.sources.iter().find(|source| source.name.value == name)
}

/// Opens `source` through `transport` at `point`, with Itaoca's stored checkpoint as its floor
/// (rule 39), and, for a resolution, refuses a point below that floor or below `lock_floor`, the
/// resolution point of the lock it replaces.
pub(crate) fn open<'t>(
    source: &ita::Source,
    transport: &'t dyn Transport,
    point: &Point,
    itaoca: Option<&Itaoca>,
    lock_floor: Option<u64>,
) -> Result<Registry<'t>, String> {
    let name = &source.name.value;
    let floor = match itaoca {
        Some(itaoca) => itaoca.load_checkpoint(name)?,
        None => None,
    };
    let registry = Registry::open(name, transport, pin_of(source)?, point, floor.as_deref())?;
    if let Point::Latest { .. } = point {
        let size = registry.point.checkpoint.size;
        if floor.as_deref().is_some_and(|stored| registry.below(stored))
            || lock_floor.is_some_and(|floor| floor > size)
        {
            return Err(format!(
                "the registry `{name}` serves a checkpoint of size {size}, below one this client already accepted; a registry never goes back"
            ));
        }
    }
    Ok(registry)
}

/// Stores the checkpoint of each registry as its floor in Itaoca (rule 39), when it is larger
/// than the one stored.
pub(crate) fn store_floors(
    registries: &BTreeMap<String, RefCell<Registry<'_>>>,
    itaoca: Option<&Itaoca>,
) -> Result<(), String> {
    let Some(itaoca) = itaoca else {
        return Ok(());
    };
    for (source, registry) in registries {
        let registry = registry
            .try_borrow()
            .map_err(|_| "a registry is read twice at once".to_owned())?;
        itaoca.store_checkpoint(source, &registry.point.document)?;
    }
    Ok(())
}

/// The lines of `ita.lock` for a registry's resolution point (rule 16).
pub(crate) fn registry_block(source: &str, pin: &str, registry: &Registry<'_>) -> String {
    format!(
        "\nregistry {source}\n  root \"{pin}\"\n  checkpoint {} \"{}\"\n",
        registry.point.checkpoint.size,
        una_project::client::hash_text(&registry.point.checkpoint.root)
    )
}

/// The `signed` form of the key the lock names for `release`: `ed25519:<owner>:<fingerprint>`
/// of its first signer by fingerprint.
pub(crate) fn signed_of(release: &LoggedRelease) -> Option<String> {
    release
        .signer()
        .map(|key| una_project::provenance::signed_name(&release.statement.name, &key.public))
}

/// The lines of `ita.lock` for a registry release (rule 16), up to its consent lines: its
/// origin, digest, key, log index and needs.
pub(crate) fn package_block(release: &LoggedRelease, signed: &str) -> String {
    let statement = &release.statement;
    let mut block = format!(
        "\npackage {} {}\n  from {}\n  digest \"{}\"\n  signed \"{signed}\"\n  logged {}\n",
        statement.name, statement.version, statement.source, statement.digest, release.index
    );
    for (source, name, version) in &statement.needs {
        block.push_str(&format!("  needs {source} {name} {version}\n"));
    }
    block
}

/// The authority a statement gives, as `Cap[op, op]` entries.
pub(crate) fn authority_of(release: &LoggedRelease) -> Vec<String> {
    release
        .statement
        .authority
        .iter()
        .map(|(capability, ops)| {
            format!(
                "{capability}[{}]",
                ops.iter().map(String::as_str).collect::<Vec<_>>().join(", ")
            )
        })
        .collect()
}

/// Whether the key of a locked release's owner changed between the lock it replaces and the new
/// resolution (rule 25): another signing key, or a key event of the owner logged after the old
/// resolution point, a recovery or a dispute that took effect included.
pub(crate) fn key_changed(
    registry: &mut Registry<'_>,
    name: &str,
    old_signed: Option<&str>,
    old_point: Option<u64>,
    new_signed: &str,
) -> Result<bool, String> {
    if old_signed.is_some_and(|old| old != new_signed) {
        return Ok(true);
    }
    let history = registry.history(una_project::client::owner_of(name))?;
    Ok(old_point.is_some_and(|point| history.changed_after(point)))
}
