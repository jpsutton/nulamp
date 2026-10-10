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
