# Third-party material shipped with Foreman

## VS Code Dark Modern for Zed

- Source: https://github.com/kevcamel/vscode_dark_modern.zed, `themes/vscode-dark-modern.json` (commit of 2026-09-06)
- In Foreman: `crates/foreman-ui/assets/themes/vscode-dark-modern.json`, compiled into `foreman.exe` as the default theme
- Licence: MIT, © 2024 Kevin Camellini. Full text: `crates/foreman-ui/assets/themes/vscode-dark-modern.LICENSE`,
  in a release `licenses\vscode-dark-modern.LICENSE`

Foreman reads the file unchanged. It adjusts one key when it loads the file, so that the contrast floors hold
(`foreman_ui::themes::fit_floors`): `info` `#BFBFBF66` becomes `#868686`.
