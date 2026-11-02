# Contributing to Supraman

## Licence

Supraman is licensed under the GNU General Public License, version 3 or (at your option) any later version
(`LICENSE`). Contributions are accepted under the same licence: what you send in is licensed to everyone under
GPL-3.0-or-later, as the rest of the code is (inbound = outbound).

## Sign-off

Every commit carries a `Signed-off-by:` line, which certifies the [Developer Certificate of Origin 1.1](https://developercertificate.org/):
that you wrote the change, or otherwise have the right to submit it under GPL-3.0-or-later.

```powershell
git commit -s
```

adds the line from your `user.name` and `user.email`. A commit without it is not merged.

## Dependencies and bundled files

A new crate must be under a licence `deny.toml` allows (`cargo deny check licenses`). After any change to
`Cargo.lock`, run `pwsh -File scripts/third-party.ps1` and commit the regenerated `THIRD_PARTY.md`;
`pwsh -File scripts/third-party.ps1 -Check` fails while it is stale. A bundled file that is not a crate (an icon,
a theme, a font) needs its licence next to it and an entry in that script.
