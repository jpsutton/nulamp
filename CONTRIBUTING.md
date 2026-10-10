# Contributing

## Branches

| Branch | Purpose |
|---|---|
| `feat/<topic>` | One new feature. Branch from `staging`, push it |
| `fix/<topic>` | One bug fix. Branch from `staging`, push it |
| `staging` | Default branch. Collects finished features and fixes through pull requests |
| `release` | Merging the `staging` → `release` pull request builds all platforms and publishes a GitHub release tagged `v<version>` |

Flow:

1. Create `feat/<topic>` or `fix/<topic>` from `staging`, commit, and push it.
2. Run `./scripts/check.sh` (format, clippy, tests) before pushing. `./scripts/fix.sh` applies
   clippy suggestions and formats.
3. Open a pull request into `staging`. CI runs a quick compile check on Linux. Merge it when it
   passes. One pull request may carry one change or gather several.
4. To release, bump `version` in the root `Cargo.toml` under `[workspace.package]` (through a pull
   request into `staging`), then open a pull request from `staging` into `release`. CI builds and
   tests it on Linux, macOS and Windows; merging it runs the release workflow, which refuses to
   run when the tag `v<version>` already exists.

CI runs only for pull requests, never on plain pushes. The full cross-platform build and tests
run once per release, on the `staging` → `release` pull request.

## Clean-room rule

Do not read Winamp source code, or the source of other classic-skin re-implementations, from anywhere. Work only from `spec/`, public format standards and third-party library documentation. Record decisions in `docs/decisions.md`.

## Commits

Use Conventional Commits (`feat:`, `fix:`, `chore:`, `docs:`, `test:`, `refactor:`).

## Tests

- `./scripts/check.sh` runs formatting, clippy and every test; it must pass before each commit.
  CI does not lint: it only builds and runs the tests on Linux, macOS and Windows.
- Golden images (spec 12 §3) live in `tests/golden/` and are produced by nulamp itself. After an
  intended visual change, regenerate them with `NULAMP_BLESS=1 cargo test -p nulamp-render --test golden`
  and review the changed PNGs in the diff. A failing comparison writes the actual image to
  `target/tmp/golden-diff/`.
- Skin corpus run: `NULAMP_SKIN_CORPUS=<folder of skins> cargo test -p nulamp-render --test corpus`.
  Every skin must load and draw. With `NULAMP_REFERENCE_DIR=<dir>` holding
  `<skin name>/main.png` screenshots of the reference player in the fixed state of 12 §3, the
  main window is compared pixel for pixel outside the song title box. Real skins and screenshots
  never go into the repository.
- Windows from Linux: `scripts/windows-check.sh` runs clippy for `x86_64-pc-windows-gnu` and the
  test suite under Wine, in Docker (`scripts/wincheck/Dockerfile`).
