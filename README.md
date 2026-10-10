# nulamp

A cross-platform audio player that plays classic Winamp 2.x skins (`.wsz`).

nulamp is an independent, clean-room implementation written from the specification in [`spec/`](spec/README.md). It is not affiliated with, and does not contain code or artwork from, the original player.

## Building

```bash
cargo build --release
```

## Checks

```bash
./scripts/check.sh
```

## Documentation

- [`spec/`](spec/README.md): the functional specification.
- [`docs/decisions.md`](docs/decisions.md): choices made where the specification is silent or deliberately changed.
