# Expanded Rotating CD

An independent WinIsland ABI v2 plugin that turns the expanded music view's album cover into a spinning circular disc. The artwork and subtle grooves rotate together once every 2.5 seconds while music is playing; there is no center hub or white arc. The plugin only draws on the expanded music page; it leaves the compact island and other expanded pages alone.

The disc overlays the native expanded album cover and slightly exceeds its bounds so the native square corners remain hidden behind a circular crop. It uses WinIsland's foreground surface and album-art API, keeping the native expanded track information and controls intact. If the host has not supplied album art, the plugin leaves the native cover visible.

## Build and install

From this directory, run:

```powershell
cargo fmt --all -- --check
cargo check --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo run --locked --example pack
```

Drop `target\Expanded Rotating CD-0.1.0.zip` onto the running island to install it. The separate package ID is `expanded-rotating-cd`, so it can be enabled alongside the compact Rotating CD plugin.

## Behavior check

1. Enable Expanded Rotating CD in WinIsland's Plugins settings.
2. Play a track with album artwork and expand the island to its music page. The expanded cover should appear circular and spin as one disc.
3. Pause playback: the disc should stop. Resume playback: it should spin again.
4. Collapse the island or navigate to a different expanded page: the overlay should disappear.

## Release

This plugin is intended for the public repository [`phonkboisad/winisland-rotatingcover-expanded`](https://github.com/phonkboisad/winisland-rotatingcover-expanded). Push a version tag matching `Cargo.toml` (currently `v0.1.0`) to run `.github/workflows/release.yml`. The pinned official PluginMarketplace workflow checks, packages, attests, and publishes a `*.winisland-plugin.zip` asset to a GitHub Release. Never replace an asset for an existing version; increment `version` in `Cargo.toml` and publish a new tag.

## Marketplace submission

After this source is public and a valid tagged release is available, use [`marketplace-registration.toml`](https://github.com/phonkboisad/winisland-rotatingcover-expanded/blob/main/marketplace-registration.toml) as the single file `plugins/expanded-rotating-cd.toml` in a pull request to [WinIsland PluginMarketplace](https://github.com/WinIslandProject/PluginMarketplace). The Marketplace contribution pull request must contain exactly that registration file; do not include plugin-source, workflow, or catalog changes in it. See the [contribution guide](https://github.com/WinIslandProject/PluginMarketplace/blob/main/CONTRIBUTING.md).
