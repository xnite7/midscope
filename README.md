# midscope

part of midnite. a lightweight valorant overlay that sits on the game's own ui.
pure rust, no injection, no game memory. it only talks to the riot client's local api and riot's own servers.

## run
1. start valorant. windowed, borderless or fullscreen-windowed all work (exclusive fullscreen can't be drawn over).
2. run `midscope.exe`. it lives in the tray.
3. **agent select**: rank, level and tags above each teammate's card, party lines underneath.
4. **the vs screen**: rank, peak and tags next to every card, party lines in the gap.
5. **in a match**: hold **tab**. rank, skins and tags join the end of the scoreboard.
6. **settings**: in the main menu, click the midnite m after the store in valorant's top bar.
7. **f6** show / hide · **f7** calibrate · **ctrl+shift+f6** quit.

## settings
`%APPDATA%\midscope\config.json` (old recon settings move over on first run).

| key | default | what it does |
|---|---|---|
| `reveal_hidden_names` | `false` | names of streamer-mode players. riot has banned accounts for this. your call |
| `show_agent_select` / `show_loading` / `show_scoreboard` | `true` | each screen on or off |
| `show_party_lines` / `show_skins` | `true` | party lines, weapon skins |
| `hold_key` | `9` (tab) | key to hold in a match |

## tags
| tag | rule |
|---|---|
| duo / trio line | your own party from riot presence; others when two players share 2+ of their last 10 matches |
| smurf | under level 50 with a plat+ peak, or 70%+ win rate over 10+ games below level 120 |
| boosted | in a party with someone 2+ ranks higher |
| tilted | 35% or lower win rate over 10+ games this act |

## how it finds things
- positions follow the game window and valorant's height-based ui scale.
- on each tab press it finds the scoreboard from its teal / red row colours and your yellow row, then matches each row's agent portrait to the players.
- the vs screen and agent select use the same portrait matching, with your yellow card pinned to you.

## look
midnite room tokens (from midnite™ couch): oled black, plum-black surfaces, satoshi, the lamp and the neon used sparingly, pills for everything you press, lowercase. team colours stay valorant's so what we add reads as part of the game.

## updates
every push to `main` builds a new version (`0.6.<build>`), signs it and publishes it as a github release. everyone's midscope checks every couple of minutes, downloads the new build, checks its signature against the key built into the exe, and swaps itself in between matches (never in agent select or mid match). a tray note says when it happened.

- one-time setup: the repo needs the `MIDSCOPE_SIGNING_KEY` secret (the secret half of `crates/midscope-core/update-key.pub`). make a new pair with `cargo run -p midscope-sign -- keygen`.
- `midscope.exe --check-update` checks once and prints what it found.

## build
```
cargo build --release
```
output: `target/release/midscope.exe` (icon and satoshi are built in).
