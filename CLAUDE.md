# sleeve

Given an album, where should the money go. Ranks ways of buying a record best-first and shows its working.

## Stack

GTK 4.22 + libadwaita 1.9 via gtk4-rs 0.11 / libadwaita-rs 0.9, Rust edition 2021 (MSRV 1.80). `gio` is a direct dependency purely to raise the API level to v2_80 — leave it.

Beyond the sibling baseline: `rapidfuzz` (search re-ranking), `rusqlite` (the cache), `toml` (config), `scraper` (one HTML table), `soup3` (all HTTP). Each is justified where it is declared in `Cargo.toml`; read that before adding a fifth.

Crate is a lib + bin so integration tests and `examples/` drive the real application rather than a copy of it.

## Commands

- `./test.sh` — fmt check, clippy with `-D warnings`, then `cargo test --all-targets`. Add `--headless` to run under Xvfb + a private D-Bus session. This is the gate; run it, not bare `cargo test`.
- **Never run `dbus-run-session` or `xvfb-run -a dbus-run-session` directly** — use `isolated-bus [--headless] -- CMD`. A private bus activates its own `xdg-document-portal`, which mounts over `/run/user/$UID/doc` and takes the login session's portal down with it when the bus exits; every flatpak on the machine then fails to launch until it is restarted. `test.sh --headless` guards against this internally, but one-off runs of a single test, or of the built binary, bypass it.
- `./install.sh` — release build, installs under `~/.local`. `./uninstall.sh` reverses it.
- `packaging/build-flatpak.sh` and `packaging/build-deb.sh` — distribution artifacts.
- `cargo run --example preview -- /tmp/preview [dark]` — paints the real pages
  offscreen to PNGs. This is how a UI change gets looked at; GNOME will not give
  a screenshot to a non-interactive caller.

No test touches the network. `test.sh` sets `GTK_A11Y=none` and `GSETTINGS_BACKEND=memory` so tests never touch real user state — keep that true for anything new.

## Layout

`src/model/` is pure logic with no GTK types. `src/ui/` is widgets and the application. Read `DESIGN.md` and `README.md` before proposing structural changes; both are current.

The seam that makes the tests possible: **`model/source/*` builds request URLs and parses response bodies; `ui/http.rs` performs the requests.** Nothing under `model/` opens a socket, so every source, every failure shape and every ranking is checkable with no display and no network. `ui/http.rs` is the only file that makes an HTTP call. Widgets report what a person did; `ui/application.rs` is the only object that mutates state or asks a source anything.

`model/cache.rs` does touch a local SQLite file and `model/settings.rs` reads one config file. Both are deterministic, neither goes near the network, and both are tested against real storage — the seam worth defending here is the network one, not the disk.

## Things that will bite

- **Ordering is lexicographic on `(tier, score)`, not numeric.** That is what makes "a purchase always outranks a stream" a property of the sort rather than an accident of the weights, and it is what stops a user's `config.toml` override from inverting the top two principles. `weights::Ordering::Numeric` exists as an opt-out and the tests assert the inversions it permits. Do not "simplify" this to a plain score sort.
- **`Outcome` has four variants and `Empty` is not `Stale`.** Half the sources are undocumented endpoints and one is an HTML table; those fail with `200 OK` and a page that no longer parses. Collapsing the two makes a broken Bandcamp reader look identical to an album Bandcamp does not stock, which silently deletes the best offer from most rankings.
- **`rapidfuzz` 0.5 is not the Python library.** `fuzz::ratio` returns 0.0–1.0, not 0–100, and there is no `token_set_ratio` or `WRatio` — `model/search.rs` builds both. `token_set_ratio` alone scores "Kid A" against "Kid A Mnesia" as a perfect match by design, which is why the blend with a plain ratio exists.
- **Region-locking removes an offer from the ranking; it is not a penalty.** There is deliberately no weight for it.
- **Bandcamp needs artist *and* title to agree independently.** A live search for "Radiohead Kid A" returns four cover versions whose titles beat the real one. `tests/fixtures/bandcamp/search-kid-a.json` is that response.
- **Qobuz needs a user token, not just an `app_id`.** The catalogue answers 401 to an app id alone; the refusal is recorded as a fixture. It is optional, because MusicBrainz already says *whether* Qobuz sells the album — the token only adds the price.
- **`musicbrainz::parse_purchase_links` is what makes this work without credentials.** `inc=url-rels` on a release yields `purchase for download` relations to Bleep, Boomkat, Presto, Beatport, Juno, Qobuz and Bandcamp — every shop that refuses an HTTP client or wants an account. Do not remove it in favour of "real" shop APIs; there are none to be had.
- **Indexed offers are not checked offers.** `Provenance` decides who wins in `offer::merge`, and a live *negative* check suppresses an indexed link. Bandcamp returning `is_purchasable: false` for an album MusicBrainz still links must not resurrect as a tier-A row.
- **Purchase links hang off a release, not a release group.** The 2000 CD has none; the digital issue has five. `look_up` asks the digital sibling when a physical pressing is chosen.
- **Discogs needs no token.** Search and release lookup both answer unauthenticated at 25 req/min; a token buys 60. The brief said otherwise and it was wrong.

## Conventions

- Use the `developing-gtk-apps` and `designing-gnome-ui` skills for widget, threading, and HIG decisions rather than deriving them again.
- Edit files with the Edit tool. Do not rewrite Rust sources through `python3 - <<PY` heredocs or `sed -i`.
- Fixtures in `tests/fixtures/` are recorded from the live APIs. When one is refreshed and a parser starts returning `Stale`, that is the fixture doing its job — fix the parser, do not re-record until it passes.
- The sibling apps (brain, familiar, magpie, planner, scribe, stickies) share this layout and these scripts; a pattern established in one is the pattern here.
