# Supraman brand files

Copied byte for byte from the Supraman suite brand kit v1 (2026-10-31), kept at
`P:\Projects\LarchSys\getsupraman\brand`; the kit's `README.md` and `index.html` are the reference for
their use. The paths here are the kit's paths. `brand_kit_copies_match_the_kit` checks the sha256 of
each against the kit when the kit folder exists (`SUPRAMAN_BRAND_KIT` names another place).

The artwork is the Supraman project's own. The text marks (`wordmarks/`, and the text in `lockups/`)
are Archivo outlined to paths; Archivo is © 2020 The Archivo Project Authors, under the SIL Open Font
License 1.1 (`source/font/OFL.txt`).

`../supraman.ico` (16, 20, 24 px from `icons/small/supraman-16.svg`; 32, 40, 48, 64, 256 px from
`icons/supraman.svg`) and the tray icons `../tray-on-dark.ico` and `../tray-on-light.ico` (from
`icons/tray/`) are rendered by `scripts/icon-gen`:

    cargo run --release --manifest-path scripts/icon-gen/Cargo.toml -- crates/foreman-app/assets/brand crates/foreman-app/assets
