# Contributing to Coucou

Thanks for wanting to help Wato grow up! 🫶

## Getting started

```bash
brew install xcodegen
cd NotchBuddy && xcodegen && open NotchBuddy.xcodeproj
```

Never edit `NotchBuddy.xcodeproj` by hand: change `project.yml` and run `xcodegen`.

Check resting island dimensions on screens with and without a notch:

```bash
bash scripts/test-screen-geometry.sh
```

## Good first contributions

- A new service integration (a poller + an entry in `PillCatalog.swift` in the `.service` category + a detail card). Look at `StripePoller.swift` for a compact example.
- A new agent: any agent already gets its own automatic pill by sending `coucou_agent` in its hook payload (see `docs/AGENTS.md`). Add an entry in `PillCatalog.swift` in the `.agent` or `.workspace` category only if you want it to be declarable in Settings → Active pills.
- A new emote or sound for Wato.
- Bug fixes — please describe how to reproduce.

## Rules of the house

- Swift 6, SwiftUI + AppKit, **no third-party dependencies** unless there's really no other way.
- Secrets go in the Keychain, never on disk or in git.
- No telemetry, no network calls except to services the user configured.
- Never block Claude Code: if the app doesn't answer, the hook must exit right away.
- Never write `~/.claude/settings.json` without a backup and the user's confirmation.
- Keep it light: 0 % CPU when the island is hidden.

## Pull requests

- One topic per PR, with a short GIF or screenshot for anything visual.
- Build must pass with no new warnings.

## Licensing

The MIT license covers the code. It does **not** cover the name "Coucou", the
character's design/look/animations, the app icon, or the sounds — those stay
Louis Raillé's, see [LICENSE-ASSETS.md](LICENSE-ASSETS.md). This fork still
uses all four (renamed to "Wato" in code and docs, but same design, same
sounds, same "Coucou" product name), so the Windows/Linux CI workflows keep
`ASSETS_LICENSED_FOR_DISTRIBUTION: 'false'` and will not upload or publish a
build, by hand or on a tag, until that's no longer true — i.e. until this fork
has its own name, icon, character design and sounds, or written permission
from the upstream author. Building and running it yourself from source is
fine in the meantime; don't flip that flag or hand out a built installer.
