//! Publishing on the client side (ADR 0122 rules 26, 37 and 43): what refuses a publication
//! before anything is built or sent, and the release statement the owner's keys sign.

use std::collections::{BTreeMap, BTreeSet};

use una_syntax::ita::{self, Origin};

use crate::version::Version;

/// Why `una ita publish` refuses the package of `manifest` (rule 43), before it checks the
/// package or builds its archive: one sentence each, U7013 where the command reports them.
pub fn refusals(manifest: &ita::Manifest) -> Vec<String> {
    let mut out = Vec::new();
    let Some(target) = manifest.publish_to.as_ref().map(|t| t.value.as_str()) else {
        return vec![
            "the head has no `publish to <source>`, so the package is never published by accident"
                .to_owned(),
        ];
    };
    if manifest.about.is_none() {
        out.push("the head has no `about`, which a published package states".to_owned());
    }
    if manifest.license.is_none() {
        out.push("the head has no `license`, which a published package states".to_owned());
    }
    for (present, section) in [
        (!manifest.sources.is_empty(), "sources"),
        (!manifest.replace.is_empty(), "replace"),
        (!manifest.trust.is_empty(), "trust"),
    ] {
        if present {
            out.push(format!(
                "a published package has no `{section}` section: only a root names {section}"
            ));
        }
    }
    for need in &manifest.needs {
        let source = match need.origin.as_ref().map(|o| &o.value) {
            None => "itaty",
            Some(Origin::Source(source)) => source.as_str(),
            Some(Origin::Path(_)) => {
                out.push(format!(
                    "`{}` is a path dependency, which a published package cannot need",
                    need.name.value
                ));
                continue;
            }
            Some(Origin::Workspace) => {
                out.push(format!(
                    "`{}` is a workspace member, which a published package cannot need",
                    need.name.value
                ));
                continue;
            }
        };
        if source != target {
            out.push(format!(
                "`{}` is needed from `{source}`; a package published to `{target}` needs only packages of `{target}`",
                need.name.value
            ));
        }
    }
    out
}

/// What a release statement says beyond its manifest (rule 37).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release<'a> {
    /// The previous entry of the name's chain, its index and leaf hash, or none.
    pub previous: Option<(u64, &'a str)>,
    /// The archive's content digest.
    pub digest: &'a str,
    /// The digest of its `ita.una`.
    pub manifest_digest: &'a str,
    /// The union of the `uses` of its public functions and handlers, and its host capabilities.
    pub authority: &'a BTreeMap<String, BTreeSet<String>>,
    /// Its trusted modules.
    pub trust: &'a BTreeSet<String>,
}

/// The text of the release statement of the package of `manifest` (rule 37), in the order and
/// form of `spec/30 §11`, its needs each of the source it is published to, by name.
pub fn statement(manifest: &ita::Manifest, release: &Release<'_>) -> Result<String, String> {
    let source = manifest
        .publish_to
        .as_ref()
        .map(|t| t.value.clone())
        .ok_or("the head has no `publish to`")?;
    let name = manifest
        .name
        .as_ref()
        .map(|n| n.value.to_string())
        .ok_or("the head has no name")?;
    let version = manifest
        .version
        .as_ref()
        .ok_or("the head has no version")
        .and_then(|v| Version::parse_full(&v.value).map_err(|_| "the version is not a full one"))?;
    let edition = manifest
        .edition
        .as_ref()
        .map(|e| e.value.clone())
        .ok_or("the head has no edition")?;
    let mut text = format!("ita-release/1\nsource {source}\nname {name}\nversion {version}\n");
    match release.previous {
        Some((index, leaf)) => text.push_str(&format!("previous {index} {leaf}\n")),
        None => text.push_str("previous none\n"),
    }
    text.push_str(&format!(
        "digest {}\nmanifest {}\nedition {edition}\n",
        release.digest, release.manifest_digest
    ));
    let mut needs: Vec<(String, Version)> = Vec::new();
    for need in &manifest.needs {
        let at_least = need
            .version
            .as_ref()
            .ok_or("a published package's needs each name a version")
            .and_then(|v| Version::parse(&v.value).map_err(|_| "a need's version does not read"))?;
        needs.push((need.name.value.to_string(), at_least.version));
    }
    needs.sort();
    for (need, version) in needs {
        text.push_str(&format!("needs {source} {need} {version}\n"));
    }
    if release.authority.is_empty() {
        text.push_str("authority none\n");
    } else {
        let caps: Vec<String> = release
            .authority
            .iter()
            .map(|(cap, ops)| {
                format!(
                    "{cap}[{}]",
                    ops.iter().cloned().collect::<Vec<_>>().join(", ")
                )
            })
            .collect();
        text.push_str(&format!("authority {}\n", caps.join(", ")));
    }
    if release.trust.is_empty() {
        text.push_str("trust none\n");
    } else {
        text.push_str(&format!(
            "trust modules {}\n",
            release.trust.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    Ok(text)
}
