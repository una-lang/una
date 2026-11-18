# The reviewed feature catalog

Note on the public tree: the scripts for validation and release, the architecture decision records, the plan and the review records are kept by the maintainers and are not in this repository; paths below that name them describe the maintainers' checkout.

`docs/features.json` is a selected, evidence-backed inventory for documentation consumers. It does not replace the specification or enumerate every language feature. Its unstable build-metadata schema is `una.features/1`.

Each record has a stable `id`, title, status, evidence edition, nullable `minimum_release`, normative source paths, and package paths relative to `tests/conformance/`. `supported` and `preview` require positive non-pending conformance cases. `edition` is the edition in which the evidence was checked, not the first edition supporting the feature. Editions 0.3, 0.4 and 0.5 retain their preview designation. Unknown release history is represented by null; assigning a minimum release requires a future extension that verifies historical evidence. `deferred` records a feature that the specification or the backlog defers, and `excluded` one that an accepted decision record rules out; neither advertises an edition or an executable fixture.

To change the reviewed set, edit the catalog, add or reuse conformance cases, and regenerate its table:

```sh
python3 -B scripts/site-features.py --editions 0.1,0.2,0.3,0.4 --write-table
cargo test -p una-cli --test feature_catalog --test conformance
```

Use the editions actually printed by the compiler built from the checkout. Python 3.11 or newer is required for its standard TOML parser. The conformance test also reads manifests through Una's own parser, confirms that the normal corpus discovers the cases, and rejects pending or negative evidence. The existing conformance runner remains responsible for exact diagnostic and runtime expectations. No new fixture format is introduced.

`site-bundle.sh` validates the front matter `editions` of every guide Markdown file present in the source snapshot, including nested directories, and every variant of `docs/std/listing.json` and `docs/std/builtins.json`, against the editions that its binary's `una --version` prints, before packing. The scan works from a source snapshot without Git metadata. It checks the catalog's positive packages with `una --json check` of that same binary; each package's `verification.lock` names the Z3 version it needs (U5003 otherwise). The generated [availability table](../guide/availability.md) must match byte for byte. Invalid data fails the build; the script does not infer support from file names, a release number, or the binary installed on the machine.

## Bundle compatibility

The bundle remains `una.site-bundle/1` and adds an optional `feature_catalog` object. The catalog source is also a listed, hashed file. There are no timestamps, host paths or locally measured results in this object. A consumer that needs feature information must check its nested schema and treat an absent catalog in an older bundle as unknown, never as proof that all features are absent.

The website source reader pinned by `site/website.lock` checks the schema and reads the version, commit, editions, neutrality and file entries; it does not read `feature_catalog`, so the field is additive for it. A consumer can read the generated Markdown table or the bundle's `feature_catalog`.
