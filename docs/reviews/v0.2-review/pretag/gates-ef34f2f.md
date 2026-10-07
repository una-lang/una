# Validation record

- Commit: `ef34f2f604657572da0c8ec97612f2e923549ef8`
- Subject: Merge PR #255: a decided argument to a function value raises the call in edition 0.1 (Closes #253)
- Run: 2026-10-07T02:44:10Z, Darwin arm64, rustc 1.98.1 (48a229cea 2026-09-01), Docker version 24.0.7, build afdd53b4e3
- Script: `scripts/release-gates.sh` at `c6d8780a2f53a8bf770493550b3467f5f719d753`

| Gate | Result | Seconds | Log sha256 |
| --- | --- | --- | --- |
| clone | PASS | 3 | `103642c321c99e45537c344843672d1d4786e867a234cc856856c74e5fda05c2` |
| build | PASS | 301 | `b4880869e4589372c3739164a87bb2274ce4f576873f4a8154c8c4bcf905b03a` |
| mac | PASS | 145 | `c35ceeaa88477417170e05620e3f4ac3723761184f0af35f36015143fd97d7e8` |
| linux | PASS | 1207 | `b61fab568d7c62f75a6d3f19edfb4b9d9b4a79db5f5977734c1fe28dcb0c8d76` |
| release | PASS | 109 | `2f3cd61fab29504cf744a0216848712f8ecb21d79f01624520d4c59b7863eaf6` |
| determinism | PASS | 56 | `70a2a4986bb5dfb460c3d2fc5f8c79431c33936e41b5ae502a200494d918bf00` |
| native | PASS | 52 | `766fbd8f2d16754c1a589045b1d04e9f3ed56204fdd4472f13e034c12cc527cd` |

## Archives

| Archive | sha256 |
| --- | --- |
| `una-0.2.0-alpha.3-aarch64-apple-darwin.tar.gz` | `a04cf3a7bba3cbbd9c52bb29797632fc960e9c365d7e9d11b7979e992448b3ee` |
| `una-0.2.0-alpha.3-x86_64-unknown-linux-gnu.tar.gz` | `a490a180b28c73850a1b9f6dc9d4bb44839287677ce0a2c31efdbbe91d57eb25` |

Verdict: PASS (every gate)
