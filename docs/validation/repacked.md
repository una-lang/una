# Repacked and withdrawn macOS archives

The macOS archives of v0.3.3 to v0.3.13 were packed again by hand on 2026-10-14 (#717), after their release, to the owners and attributes that releases have from v0.3.14 on (#679): each entry owned by `0:0` with no user or group name, and no extended attributes. The archive that each release publishes is therefore not the one its gate run built, and its digest is not the one its validation summary lists. Each pair below was checked before the repacked archive was published: the same entries, modes, times and file bytes, owners `0:0`, and no extended attributes. `install.sh` accepts a repacked archive only when its digest is the published one that this table pairs with that summary's digest, for that archive's name.

| Archive | Gated sha256 | Published sha256 | Check |
| --- | --- | --- | --- |
| `una-0.3.3-aarch64-apple-darwin.tar.gz` | `1b07abb06142d43963b8a4d30b4a4d031b9527c18a690538249a4b25542cf59f` | `93faa1e7006aff837255c544e96b9c49a7996fb0ce4c06818d72ff05a6a4b583` | entries, modes, times and file bytes identical; owners 0:0; no extended attributes |
| `una-0.3.4-aarch64-apple-darwin.tar.gz` | `7c9f4db99bc1b03689b164a351454a6457d635bf34a35f983b04b29f6c5b1b29` | `bf6847a425518477ff15370852e3bcb4271d9046529ce27c87789d4b6a6f0c8c` | entries, modes, times and file bytes identical; owners 0:0; no extended attributes |
| `una-0.3.5-aarch64-apple-darwin.tar.gz` | `2a2f1c5132031519f26bb30bedcf166d726b96bddde41bc51bb54d99311ed4de` | `55f1e4130cf44d95913ca996bec756345b4a844a1432a1ed81aa323baf19d822` | entries, modes, times and file bytes identical; owners 0:0; no extended attributes |
| `una-0.3.6-aarch64-apple-darwin.tar.gz` | `d2ab14161a92ac2d3447ff6c8a2ca9144c2f092b86a2f8ba89286cd60fb06078` | `16c8b7c6f757f497da97ce5c21bc5d1ddd76e1794d5bb3bcab6a64c69dc55bd5` | entries, modes, times and file bytes identical; owners 0:0; no extended attributes |
| `una-0.3.7-aarch64-apple-darwin.tar.gz` | `98a175345677d5c26c234baa74ab5f1a48b7b480d7a15f80e662dc2dc7a23495` | `7b501cb3012cf39ccf06102471d6c97d16af23792dd7732a38c854c56a46da10` | entries, modes, times and file bytes identical; owners 0:0; no extended attributes |
| `una-0.3.8-aarch64-apple-darwin.tar.gz` | `01283092e8094748b3ae951ad378e8d9997eebb202a82e59c5ac212374d701d9` | `9b4389c1e025d2793f23670b6f8ec251e5d68660fa98611990562bfbacd7ed25` | entries, modes, times and file bytes identical; owners 0:0; no extended attributes |
| `una-0.3.9-aarch64-apple-darwin.tar.gz` | `99a045ad4634b6be889de3294528eb5a71fcb64a1d01ebdbe21d4bfaae896362` | `7733f8e2cb619db29190ff41c3d6227cfba51d3e7e1f4a38102e7d141085f78c` | entries, modes, times and file bytes identical; owners 0:0; no extended attributes |
| `una-0.3.10-aarch64-apple-darwin.tar.gz` | `fc88ea034bbf2dc3f3883b874a67cdc56f33f96d1e48d82a820bec8b8cddfd00` | `4679e4fcbe1caad3029f9cbe09b1380a4b39296dbf5200e4e36ba25d7adcf508` | entries, modes, times and file bytes identical; owners 0:0; no extended attributes |
| `una-0.3.11-aarch64-apple-darwin.tar.gz` | `bdd83d26939e798de02164e4c1ae64c2ba86d26bee6aa4554d2d1d6aea9dae5b` | `71f230b899e48917d1ec40eaf98664eedd946aeb2f20741933d25cfd6e5b56b9` | entries, modes, times and file bytes identical; owners 0:0; no extended attributes |
| `una-0.3.12-aarch64-apple-darwin.tar.gz` | `36707b8b9d81429dbb72410244046c9a4c5bf7d1df1ed55d918d90defb97bc82` | `04c93e89013e09275fb58e3469947bf784b05919228f3e8af5c602d406496c78` | entries, modes, times and file bytes identical; owners 0:0; no extended attributes |
| `una-0.3.13-aarch64-apple-darwin.tar.gz` | `aa688976b5ee1b2427f3478bb2afe72882a443576e740b91092e547d801b7021` | `0d424945c1bca0708b47de4d4e7bd5749d7d1a88152d8c2d1dd389fe04222ea5` | entries, modes, times and file bytes identical; owners 0:0; no extended attributes |

## Withdrawn

The macOS archives of v0.3.0, v0.3.1 and v0.3.2 were withdrawn and are not published. `install.sh` says so instead of failing on a download; on macOS, install v0.3.3 or later.

| Archive |
| --- |
| `una-0.3.0-aarch64-apple-darwin.tar.gz` |
| `una-0.3.1-aarch64-apple-darwin.tar.gz` |
| `una-0.3.2-aarch64-apple-darwin.tar.gz` |
