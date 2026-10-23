# The reviewed feature catalog

`docs/features.json` is a selected, evidence-backed inventory for documentation consumers. It does not replace the specification or enumerate every language feature. Its unstable build-metadata schema is `una.features/1`.

Each record has a stable `id`, title, status, evidence edition, nullable `minimum_release`, normative source paths, and package paths relative to `tests/conformance/`. `supported` and `preview` require positive non-pending conformance cases. `edition` is the edition in which the evidence was checked, not the first edition supporting the feature. Editions 0.3 and 0.4 retain their preview designation. Unknown release history is represented by null; assigning a minimum release requires a future extension that verifies historical evidence. Proposed and excluded records advertise no supported edition or executable fixture.

To change the reviewed set, edit the catalog, add or reuse conformance cases, and regenerate its table:

```sh
python3 -B scripts/site-features.py --editions 0.1,0.2,0.3,0.4 --write-table
cargo test -p una-cli --test feature_catalog --test conformance
```

Use the editions actually printed by the compiler built from the checkout. Python 3.11 or newer is required for its standard TOML parser. The conformance test also reads manifests through Una's own parser, confirms that the normal corpus discovers the cases, and rejects pending or negative evidence. The existing conformance runner remains responsible for exact diagnostic and runtime expectations. No new fixture format is introduced.

`site-bundle.sh` validates every tracked guide front-matter edition and every standard-library variant against the editions of its binary, before packing. It checks the catalog's positive packages with that same binary and the pinned Z3. The generated [availability table](../guide/availability.md) must match byte for byte. Invalid data fails the build; the script does not infer support from file names, a release number, or the binary installed on the machine.

## Bundle compatibility

The bundle remains `una.site-bundle/1` and adds an optional `feature_catalog` object. The catalog source is also a listed, hashed file. There are no timestamps, host paths or locally measured results in this object. A consumer that needs feature information must check its nested schema and treat an absent catalog in an older bundle as unknown, never as proof that all features are absent.

The existing website source reader pinned by `site/website.lock` accepts additive fields and continues reading its existing version, editions and file entries. It ignores the optional catalog; this change does not promise a new website navigation route. Consumers can read the generated Markdown and machine-readable bundle data. A future website UI can opt into the catalog without a schema break. The website lock is not moved here.

The compatibility check loads the pinned source reader with both a legacy bundle and the extended bundle, checking identical release metadata and file hashes. Build two bundles of the same commit and compare their archive hashes. This remains separate from the release gate's full website build and samples check.
