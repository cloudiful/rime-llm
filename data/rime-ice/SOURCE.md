# Bundled Dictionary Source

This directory contains a vendored snapshot of the [rime-ice](https://github.com/iDvel/rime-ice) dictionary data.

- Upstream: `https://github.com/iDvel/rime-ice`
- Snapshot commit: `b681a34f788795034b3b288830f4861980bc8b0d`
- Snapshot date: `2026-07-30`
- Upstream license: `GPL-3.0-only`

The snapshot contains the manifest and the five tables enabled by that manifest:

- `rime_ice.dict.yaml`
- `cn_dicts/8105.dict.yaml`
- `cn_dicts/base.dict.yaml`
- `cn_dicts/ext.dict.yaml`
- `cn_dicts/tencent.dict.yaml`
- `cn_dicts/others.dict.yaml`

The optional `41448` large character table is not included because it is disabled in the upstream manifest and is not needed for normal simplified Chinese input.

## Credits

Credit for this dictionary belongs to iDvel and the rime-ice contributors. The upstream README acknowledges `@Huandeep` and `@Lithium-7`, and the vendored files retain their original attribution comments, including the Tencent word-vector source.

The dictionary files in this directory are not relicensed under this project's Apache-2.0 license. Keep this file and `LICENSE` when redistributing them, and comply with GPL-3.0-only for the dictionary data.

## Updating

To update the bundled snapshot, select an upstream commit and run `scripts/update-rime-ice.sh <40-character-commit>`. The script downloads only the files listed above, updates this source record, and refreshes the SHA-256 values. Keep the manifest and its enabled tables in sync. Run `cargo fmt -- --check` and `cargo test --features metal` after the update.

Current SHA-256 values:

```text
30fa643eec7af585cffacdd9bb6556ce4d17a0e848b83ea1fca3fdcfc5891595  rime_ice.dict.yaml
ddad7554a5bdecbbeb557ee703ecee548c828722a262ec9e5aee9caad8e52cf8  cn_dicts/8105.dict.yaml
3bbf598226fda9b629a7aa0c141ca20601a2c3e102562b3fb7322babf1ca56ba  cn_dicts/base.dict.yaml
543859f891dec5335b831840d895e1a1c4ef500648aef52b5e7c2963a2a2d256  cn_dicts/ext.dict.yaml
c962190fdebdd1d388a6ac7a81a9c2f7002e79b0675f440fbd8a80b1e890adde  cn_dicts/tencent.dict.yaml
6a6b1a77d94c7cdf9203cf426e67f350215d2d73259fe3769c97d2a18f521c28  cn_dicts/others.dict.yaml
3972dc9744f6499f0f9b2dbf76696f2ae7ad8af9b23dde66d6af86c9dfb36986  LICENSE
```
