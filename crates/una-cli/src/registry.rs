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

use std::collections::{BTreeMap, BTreeSet};

use una_diag::Code;
use una_project::ProjectError;
use una_project::client::{Floors, LoggedRelease, OpenError, Point, Registry};
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

/// The places of `source`, in the order they are tried: its mirrors, then its own URL. Each
/// place reads, behind it, from the other places that have a transport, for any document it
/// omits below its checkpoint ([`transport::Fallback`]).
pub(crate) fn places_of(source: &ita::Source, mirrors: &Mirrors) -> Vec<Place> {
    let name = &source.name.value;
    let own = &source.url.value;
    let mut locations: Vec<(String, Result<Location, String>)> = mirrors
        .of(name)
        .iter()
        .map(|mirror| (mirror.url.clone(), Ok(mirror.location.clone())))
        .collect();
    locations.push((own.clone(), Location::parse(own)));
    let transports = |except: usize| -> Vec<Box<dyn Transport>> {
        locations
            .iter()
            .enumerate()
            .filter(|(at, _)| *at != except)
            .filter_map(|(_, (_, location))| location.as_ref().ok())
            .filter_map(|location| transport_for(location).ok())
            .collect()
    };
    locations
        .iter()
        .enumerate()
        .map(|(at, (from, location))| {
            let first = location
                .clone()
                .and_then(|location| transport_for(&location));
            let place = first.map(|first| -> Box<dyn Transport> {
                Box::new(transport::Fallback::new(first, transports(at)))
            });
            (from.clone(), place)
        })
        .collect()
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
    lock_floor: Option<&(u64, String)>,
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
        // Nothing served is a refusal of service; anything served that fails is evidence. The
        // registry client tells the two apart by where the failure arose, never by its text.
        open(source, transport, point, itaoca, lock_floor).map_err(|error| match error {
            OpenError::Unavailable(why) | OpenError::Omitted(_, why) => Attempt::Unavailable(why),
            OpenError::Refused(why) => Attempt::Refused(why),
        })
    });
    let mut notes: Vec<String> = fetched
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
        Some(((registry, opened), _)) => {
            notes.extend(opened);
            Ok((registry, notes))
        }
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

/// The code of a failure `registry` just met (`spec/30 §14` rule 1): U7013 when a document
/// that no place serves made the result unavailable, U7010 when what was served does not
/// verify. Read from the registry's record of omissions, never from the message.
pub(crate) fn code_of(registry: &mut Registry<'_>) -> Code {
    if registry.take_omitted().is_empty() {
        Code::U7010
    } else {
        Code::U7013
    }
}

/// A witness time, seconds since the Unix epoch, as a UTC date and time:
/// `YYYY-MM-DDTHH:MM:SSZ` (the proleptic Gregorian calendar, by days since the epoch).
pub(crate) fn utc(seconds: u64) -> String {
    let days = seconds / 86_400;
    let rest = seconds % 86_400;
    // Days since the epoch to a civil date, by eras of 400 years from March 1, year 0.
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
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

/// Opens `source` through `transport` at `point`, with Itaoca's floors (rule 39), kept under
/// the registry's identity ([`una_project::itaoca::registry_id`], from the pin): its stored
/// checkpoint, which the point must extend and, for a resolution, not fall below, and its
/// highest root version accepted. For a resolution, the point must also extend `lock_floor`,
/// the resolution point of the lock it replaces, by its size and root hash. The floors are
/// stored as soon as the point is accepted, whatever the command then writes; a floor that
/// cannot be stored is a note, never a failure. Returns the registry and its notes.
pub(crate) fn open<'t>(
    source: &ita::Source,
    transport: &'t dyn Transport,
    point: &Point,
    itaoca: Option<&Itaoca>,
    lock_floor: Option<&(u64, String)>,
) -> Result<(Registry<'t>, Vec<String>), OpenError> {
    let name = &source.name.value;
    let pin = pin_of(source).map_err(OpenError::Refused)?;
    let id = una_project::itaoca::registry_id(pin);
    let mut notes = Vec::new();
    // A floor that cannot be read is no floor; it is said, and the place is still verified.
    let (floor, root_version) = match itaoca {
        Some(itaoca) => (
            itaoca.load_checkpoint(&id).unwrap_or_else(|why| {
                notes.push(format!("note: the floor of `{name}` does not read: {why}"));
                None
            }),
            itaoca.load_root_version(&id).unwrap_or_else(|why| {
                notes.push(format!("note: the root floor of `{name}` does not read: {why}"));
                None
            }),
        ),
        None => (None, None),
    };
    let floors = Floors {
        checkpoint: floor.as_deref(),
        root_version,
    };
    let mut registry = Registry::open_with(name, transport, pin, point, &floors)?;
    if matches!(point, Point::Latest { .. } | Point::Served) {
        let size = registry.point.checkpoint.size;
        if floor.as_deref().is_some_and(|stored| registry.below(stored)) {
            return Err(OpenError::Refused(format!(
                "the registry `{name}` serves a checkpoint of size {size}, below one this client already accepted; a registry never goes back"
            )));
        }
        if let Some((size, hash)) = lock_floor {
            registry
                .extends(*size, hash)
                .map_err(OpenError::Refused)?;
        }
    }
    if let Some(itaoca) = itaoca {
        let document = registry.point.document.clone();
        let version = registry.root.version;
        let mut consistent = |stored: &str| registry.consistent_with(stored);
        if let Err(why) = itaoca.store_checkpoint(&id, &document, &mut consistent) {
            notes.push(format!(
                "note: the checkpoint of `{name}` was not stored as its floor: {why}"
            ));
        }
        if let Err(why) = itaoca.store_root_version(&id, version) {
            notes.push(format!(
                "note: the root version of `{name}` was not stored as its floor: {why}"
            ));
        }
    }
    Ok((registry, notes))
}

/// The lines of `ita.lock` for a registry's resolution point (rule 16).
/// With `accepted`, per owner, the recoveries and disputes pending at the point whose pending
/// state the lock records as consented to, `accepted <owner> <index> …`; then with `effects`,
/// those in effect at the point whose taking effect it records as consented to,
/// `effect <owner> <index> …`; owners in order and indices ascending.
pub(crate) fn registry_block(
    source: &str,
    pin: &str,
    registry: &Registry<'_>,
    accepted: &BTreeMap<String, BTreeSet<u64>>,
    effects: &BTreeMap<String, BTreeSet<u64>>,
) -> String {
    let mut block = format!(
        "\nregistry {source}\n  root \"{pin}\"\n  checkpoint {} \"{}\"\n",
        registry.point.checkpoint.size,
        una_project::client::hash_text(&registry.point.checkpoint.root)
    );
    for (owner, indices) in accepted.iter().filter(|(_, indices)| !indices.is_empty()) {
        let list: Vec<String> = indices.iter().map(u64::to_string).collect();
        block.push_str(&format!("  accepted {owner} {}\n", list.join(" ")));
    }
    for (owner, indices) in effects.iter().filter(|(_, indices)| !indices.is_empty()) {
        let list: Vec<String> = indices.iter().map(u64::to_string).collect();
        block.push_str(&format!("  effect {owner} {}\n", list.join(" ")));
    }
    block
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
/// resolution (rule 25): another signing key, or any entry of the owner's chain logged at or
/// after the old resolution point, a recovery or a dispute that took effect included, except
/// the entries `accepted`, whose consent the lock records.
pub(crate) fn key_changed(
    registry: &mut Registry<'_>,
    name: &str,
    old_signed: Option<&str>,
    old_point: Option<u64>,
    new_signed: &str,
    accepted: &BTreeSet<u64>,
    effects: &BTreeSet<u64>,
) -> Result<bool, String> {
    if old_signed.is_some_and(|old| old != new_signed) {
        return Ok(true);
    }
    let history = registry.history(una_project::client::owner_of(name))?;
    Ok(old_point.is_some_and(|point| history.changed_after_except(point, accepted, effects)))
}
