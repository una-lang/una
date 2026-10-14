// Compares the heading ids of scripts/doc-links.py with the ones the website gives (#708), so a
// fragment that the link check accepts is an id of the published page. Run by
// scripts/site-website.sh in the website's clone, after its packages are installed:
//   node website-anchors.mjs IDS ROOT
// IDS is the output of `scripts/doc-links.py --anchors`, ROOT the una checkout it read. Each
// document is parsed with the website's own Markdown setup and "anchors" rule, with the
// specification's numbered anchors in its chapters, and its ids must be the same, in order.
import { readFileSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

const [idsFile, root] = process.argv.slice(2);
if (!idsFile || !root) {
  console.error("usage: node website-anchors.mjs IDS ROOT");
  process.exit(2);
}
const module = path.resolve("scripts/preview/markdown.mjs");
const { createMarkdown } = await import(pathToFileURL(module).href).catch((e) => {
  console.error(`website anchors: FAIL (the website has no ${module}: ${e.message})`);
  process.exit(1);
});
const md = createMarkdown();
const expected = JSON.parse(readFileSync(idsFile, "utf8"));
const failures = [];
let count = 0;
for (const [file, { spec, ids }] of Object.entries(expected)) {
  const env = { specAnchors: spec };
  md.parse(readFileSync(path.join(root, file), "utf8"), env);
  const got = (env.headings || []).map((h) => h.id);
  count += got.length;
  if (JSON.stringify(got) !== JSON.stringify(ids)) {
    const at = got.findIndex((id, i) => id !== ids[i]);
    const i = at < 0 ? Math.min(got.length, ids.length) : at;
    failures.push(`${file}: heading ${i + 1} is ${JSON.stringify(got[i])} on the website, ${JSON.stringify(ids[i])} in doc-links`);
  }
}
if (failures.length) {
  console.error(`${failures.join("\n")}\nwebsite anchors: FAIL (${failures.length} documents)`);
  process.exit(1);
}
console.log(`website anchors: PASS (${count} ids in ${Object.keys(expected).length} documents)`);
