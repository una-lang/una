//! The registry client as `una ita` uses it (ADR 0122 rules 25, 26, 34 to 40, `spec/30 §14`):
//! where each source of the root is read from, opening it verified, and the lines a lock
//! writes for a release.
//!
//! **Where a source is read from** (rule 40). Each source is read from its mirrors, the
//! locations Itaoca's `mirrors.una` lists for it, in order, then from its own URL, the one
//! `ita.una`'s `sources` gives it; the first place whose copy of the root metadata and the
//! checkpoint verifies is used ([`una_project::mirrors::first_verified`]). A mirror that serves a
//! copy that does not verify is skipped and named (U7010). Every location goes through
//! [`Location::parse`]: plain `http://` is refused, and an `https://` location has no transport
//! in this release, which the diagnostic says. A source with no place that verifies is U7010,
//! naming each place that served a bad copy, or U7013 when none could serve at all. A mirror is
//! verified as the source itself: the root from the pin of `sources`, the checkpoint and its
//! witnesses, every proof and signature, and every archive's digest.
//!
//! `itaty`, whose root metadata is to be built into `una`, is not served by this release: a need
//! of it stays U7013. No program is started and no network connection is opened.

use std::cell::RefCell;
use std::collections::BTreeMap;

use una_diag::Code;
use una_project::ProjectError;
use una_project::client::{LoggedRelease, Point, Registry};
use una_project::itaoca::Itaoca;
use una_project::mirrors::{self, Attempt, Mirrors};
use una_project::transport::{self, Location, Transport};
use una_syntax::ita;

/// A place a source may be read from: its location as written, and its transport, or why it
/// has none.
pub(crate) type Place = (String, Result<Box<dyn Transport>, String>);

/// The mirrors of Itaoca's `mirrors.una` (rule 40), none when Itaoca has none or no file. A file
/// that does not read, or that `mirrors::parse` refuses, is U7008 at the start of `ita.una`,
/// naming the line of `mirrors.una`, so no source is read from a list that is not the one
/// written.
pub(crate) fn load_mirrors(itaoca: Option<&Itaoca>) -> Result<Mirrors, Vec<ProjectError>> {
    let refuse = |message: String| ProjectError {
        code: Code::U7008,
        range: 0..0,
        message,
    };
    let Some(itaoca) = itaoca else {
        return Ok(Mirrors::default());
    };
    let text = match itaoca.load_mirrors() {
        Ok(Some(text)) => text,
        Ok(None) => return Ok(Mirrors::default()),
        Err(why) => return Err(vec![refuse(format!("Itaoca's mirrors.una does not read: {why}"))]),
    };
    let (read, errors) = mirrors::parse(&text);
    if errors.is_empty() {
        return Ok(read);
    }
    Err(errors
        .into_iter()
        .map(|error| {
            let line = text
                .get(..error.range.start)
                .map_or(1, |head| head.matches('\n').count() + 1);
            refuse(format!(
                "Itaoca's mirrors.una, line {line}: {}",
                error.message
            ))
        })
        .collect())
}

/// The transport of a location: a directory's; an `https://` location has none in this release.
fn transport_for(location: &Location) -> Result<Box<dyn Transport>, String> {
    match location {
        Location::Https(url) => Err(format!(
            "`{url}` is reached over https, and this release has no https transport"
        )),
        Location::Directory(_) => Ok(transport::open(location)),
    }
}

/// The places of `source`, in the order they are tried: its mirrors, then its own URL.
pub(crate) fn places_of(source: &ita::Source, mirrors: &Mirrors) -> Vec<Place> {
    let name = &source.name.value;
    let mut places: Vec<Place> = mirrors
        .of(name)
        .iter()
        .map(|mirror| (mirror.url.clone(), transport_for(&mirror.location)))
        .collect();
    let own = &source.url.value;
    places.push((
        own.clone(),
        Location::parse(own).and_then(|location| transport_for(&location)),
    ));
    places
}

/// Opens `source` at `point` from the first of `places` whose root metadata and checkpoint
/// verify ([`open`]), and returns it with a note per mirror skipped for a copy that did not
/// verify. With none: U7010 naming each place that served a bad copy, or U7013 when no place
/// could serve, with how to list a mirror.
pub(crate) fn open_first<'t>(
    source: &ita::Source,
    places: &'t [Place],
    point: &Point,
    itaoca: Option<&Itaoca>,
    lock_floor: Option<u64>,
) -> Result<(Registry<'t>, Vec<String>), (Code, String)> {
    let name = &source.name.value;
    let refs: Vec<mirrors::Place<'t>> = places
        .iter()
        .map(|(from, transport)| {
            (
                from.clone(),
                transport
                    .as_ref()
                    .map(|transport| transport.as_ref())
                    .map_err(Clone::clone),
            )
        })
        .collect();
    let fetched = mirrors::first_verified(&refs, &mut |transport| {
        open(source, transport, point, itaoca, lock_floor).map_err(|why| {
            // Nothing served is a refusal of service; anything served that fails is evidence.
            if why.contains("has no `")
                || why.contains("no root metadata has the pinned digest")
                || why.contains("cannot read")
            {
                Attempt::Unavailable(why)
            } else {
                Attempt::Refused(why)
            }
        })
    });
    let notes: Vec<String> = fetched
        .refused
        .iter()
        .map(|skipped| {
            format!(
                "note: U7010: `{}` served a copy of `{name}` that does not verify, and was skipped: {}",
                skipped.from, skipped.reason
            )
        })
        .collect();
    match fetched.value {
        Some((registry, _)) => Ok((registry, notes)),
        None if !fetched.refused.is_empty() => Err((
            Code::U7010,
            format!(
                "no place of the source `{name}` serves a copy that verifies: {}",
                fetched
                    .refused
                    .iter()
                    .map(|s| format!("`{}`: {}", s.from, s.reason))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        )),
        None => Err((
            Code::U7013,
            format!(
                "the source `{name}` has no place this release can read: {}; list a mirror directory for `{name}` in Itaoca's mirrors.una, as a `file://` URL",
                fetched
                    .unavailable
                    .iter()
                    .map(|s| format!("`{}`: {}", s.from, s.reason))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        )),
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
