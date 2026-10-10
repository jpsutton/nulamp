# Contributing

## Branches

| Branch | Purpose |
|---|---|
| `feat/<topic>` | One new feature. Branch from `staging` |
| `fix/<topic>` | One bug fix. Branch from `staging` |
| `staging` | Collects finished feature and fix branches. CI runs on every push |
| `release` | Each push builds all platforms and publishes a GitHub release tagged `v<version>` |
| `main` | Follows `release` |

Flow:

1. Create `feat/<topic>` or `fix/<topic>` from `staging`.
2. Run `./scripts/check.sh` (format, clippy, tests). `./scripts/fix.sh` applies clippy suggestions and formats.
3. Merge into `staging` (pull request, or a merge commit).
4. To release, bump `version` in the root `Cargo.toml` under `[workspace.package]`, then merge `staging` into `release`. The release workflow refuses to run when the tag `v<version>` already exists.
5. Fast-forward `main` to `release`.

## Clean-room rule

Do not read Winamp source code, or the source of other classic-skin re-implementations, from anywhere. Work only from `spec/`, public format standards and third-party library documentation. Record decisions in `docs/decisions.md`.

## Commits

Use Conventional Commits (`feat:`, `fix:`, `chore:`, `docs:`, `test:`, `refactor:`).

## Tests

- `./scripts/check.sh` runs formatting, clippy and every test; it must pass before each commit.
- Golden images (spec 12 §3) live in `tests/golden/` and are produced by nulamp itself. After an
  intended visual change, regenerate them with `NULAMP_BLESS=1 cargo test -p nulamp-render --test golden`
  and review the changed PNGs in the diff. A failing comparison writes the actual image to
  `target/golden-diff/`.
- Skin corpus run: `NULAMP_SKIN_CORPUS=<folder of skins> cargo test -p nulamp-render --test corpus`.
  Every skin must load and draw. With `NULAMP_REFERENCE_DIR=<dir>` holding
  `<skin name>/main.png` screenshots of the reference player in the fixed state of 12 §3, the
  main window is compared pixel for pixel outside the song title box. Real skins and screenshots
  never go into the repository.
