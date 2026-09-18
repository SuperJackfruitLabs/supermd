# Vendored base16 scheme sources

These are unmodified copies of scheme files from
[`tinted-theming/schemes`](https://github.com/tinted-theming/schemes),
taken from the `spec-0.11` branch at commit
`7cda828e3ed8bca190857bdcfe6bac84db690780` (2026-09-12). Upstream
publishes 340 schemes in one machine-readable format; these are the 24
SuperMD uses. They are MIT licensed — see `LICENSE`, copied from the
same commit.

They are vendored rather than fetched so the build is reproducible
offline and the attribution is honest, the same reason `vendor/` holds
a patched crate.

## What uses them

`cargo run --example import_base16` reads the twenty **converted**
schemes below and writes `assets/themes/<scheme>.toml`. The generated
files are committed; a test in `src/base16.rs` regenerates them and
fails if a committed file has drifted from its source.

Converted (each ships as a theme):

    ayu-dark              ayu-light             catppuccin-frappe
    catppuccin-latte      catppuccin-macchiato  catppuccin-mocha
    dracula               everforest            github
    kanagawa              monokai               one-light
    onedark               rose-pine             rose-pine-dawn
    rose-pine-moon        tokyo-night-dark      tokyo-night-light
    tokyo-night-storm     zenburn

The other four are **fixtures**, not themes. SuperMD already ships
hand-tuned `nord`, `gruvbox-dark`, `solarized-dark` and
`solarized-light` written before the converter existed; the same four
schemes exist upstream, so the converter is run against them and its
output compared with what a human chose. They are the only check on the
mapping that is not the mapping's own opinion. No theme file is
generated from them and the hand-written four are never overwritten:

    nord                  gruvbox-dark-hard
    solarized-dark        solarized-light

## Updating

Re-download the chosen files at a new upstream commit, update the
commit recorded above, run `cargo run --example import_base16`, and
run the tests. Any scheme whose palette moved shows up as a changed
`assets/themes/*.toml`, and the contrast guards in `src/theme.rs` are
what decide whether the new values are shippable.
