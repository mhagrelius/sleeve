# Sleeve

Search an album, pick the right pressing, and find out **where to spend your money** — ranked best-first, with the reasoning shown.

This is not a "where can I stream it" tool.

```
┌─ Blade Runner ──────────────────────────────────────────── ☰ ─┐
│                                                               │
│  ┌────────┐  Blade Runner                                     │
│  │  ▣ ◎   │  Vangelis                                         │
│  └────────┘  1994-06-10 · GB · CD · 12 tracks · East West     │
│                                                               │
│  ┃ Buy it                                                     │
│  ┃ Buy it from Bandcamp for £11.00. 24-bit/96 kHz FLAC — and  │
│  ┃ you keep the files. Watch which master you get: the 2017   │
│  ┃ remaster measures DR6, against DR13 for the 1994 original. │
│  ┃ CD (used) is cheaper at £4.50. Not checked: Discogs.       │
│                                                               │
│  Ranked                                                       │
│  ┌───────────────────────────────────────────────────────┐    │
│  │ Ⓐ 1. Bandcamp      24-bit/96 kHz FLAC · £11.00   130 ⌄│    │
│  │ Ⓐ 2. Qobuz Store   24-bit/192 kHz · 2017 remaster 107 ⌄│   │
│  │ Ⓐ 3. CD (used)     16-bit/44.1 kHz CD · £4.50    102 ⌄│    │
│  │ Ⓑ 4. iTunes Store  AAC 256 · £7.99                50 ⌄│    │
│  │ Ⓒ 5. Qobuz         24-bit/192 kHz FLAC            41 ⌄│    │
│  └───────────────────────────────────────────────────────┘    │
└───────────────────────────────────────────────────────────────┘
```

## How it ranks

Four principles, in strict priority order:

1. **Owning DRM-free files beats renting.** A purchase you keep forever outranks any subscription stream, always. Catalogues get pulled; files don't.
2. **Higher artist payout beats lower.** Direct-to-artist > hi-res store > premium-only streaming > freemium streaming.
3. **Master quality beats format specs.** A 16-bit/44.1 kHz copy of a good master beats 24-bit/192 kHz of a brickwalled one. Bit depth and sample rate are tiebreakers only.
4. **Actually available beats theoretically ideal.**

| Tier | | Base |
|---|---|---|
| **A** | DRM-free purchase, lossless | 100 |
| **B** | DRM-free purchase, lossy | 50 |
| **C** | Subscription streaming, lossless | 30 |
| **D** | Subscription streaming, lossy | 10 |
| **E** | Not legitimately available | — |

The list is sorted by **tier first, then score**. That is what makes principles 1 and 2 guarantees rather than arithmetic that happens to work out — no weight, including one you write yourself, can put a stream above a purchase. Set `ordering = "numeric"` in the config if you want the raw score to decide instead; the tests document the inversions that permits.

Every row expands to show the arithmetic that placed it:

```
Tier A — Own it, lossless               +100
Roughly 82–90% reaches the artist        +15
Lossless                                  +5
24-bit                                    +2
DR13                                      +8
Total                                    130
```

Price is reported but never scored. When the best option and the cheapest one differ, the recommendation says so.

## Finding the right record

Album and artist names collide constantly — soundtracks and classical worst of all, but so does any band with a self-titled record.

- **Fuzzy on both fields.** Typos, missing subtitles and wrong-but-close artist names still find it. MusicBrainz relevance is the primary signal, re-ranked locally against every alias, legal name and non-Latin-script form the artist has.
- **Candidates, never a guess.** You get a ranked list with year, type, track count, cover art and MusicBrainz's own disambiguation comment. When several fit equally well, the page says so instead of leading with one.
- **Near misses in their own section.** Other records by the same artist, and similarly-titled records by other artists — kept visually apart from the confident matches, because that is often how you find the edition you actually wanted.
- **Album, then pressing.** A release group holds the original, the remaster, the deluxe edition and the vinyl reissue. Those are different products with different masters, so you drill into them rather than getting a flattened list.

## Sources

**Sleeve needs no accounts, no API keys and no sign-ups.** Run it with an empty
config and every source below works.

| | Needs | |
|---|---|---|
| **MusicBrainz** | — | Identity, editions, pressings — **and purchase links to every shop** |
| **Bandcamp** | — | Price and availability at the highest-payout source there is |
| **iTunes Store** | — | Purchase price, and a fallback cover |
| **Odesli** | — | Streaming availability everywhere at once |
| **Discogs** | — | Physical pressings and marketplace prices |
| **Dynamic Range DB** | — | Which master is which |
| **Cover Art Archive** / **Deezer** | — | Sleeves |

The shops with no usable public API — **Bleep**, **Boomkat**, **Presto**,
**Beatport**, **Juno**, **HDtracks**, and **Qobuz**, which wants a paid
subscription before its own API will answer — are reached through MusicBrainz's
`purchase for download` relations. Editors record where an album is sold, so a
shop that refuses an HTTP client still shows up as a ranked tier-A row with a
link. It carries no price, which costs nothing: price is reported, never scored.

That is an *index*, not a check. Coverage depends on an editor having added the
link, and Sleeve marks those rows accordingly. Where a shop's own API also
answers, the live check wins — including when it says *no*. Bandcamp reports
`is_purchasable: false` for albums MusicBrainz still lists a Bandcamp link for,
and the live answer removes the row rather than the index resurrecting it.

Two optional keys, both of which only *add* to the answer:

| | Buys you |
|---|---|
| `discogs_token` | 60 requests a minute instead of 25. Nothing else. |
| `qobuz_user_token` | A price on the Qobuz row. Whether Qobuz sells it is already known. |

A source that fails, rate-limits or is unconfigured leaves a visible **"not
checked"** row. It never quietly shrinks the answer — a partial answer that looks
whole is worse than one that admits it.

Genuinely absent: **7digital**, whose API moved behind a commercial partner
agreement when Songtradr acquired them, and which no index can price.

## Install

```sh
./install.sh          # builds release, installs under ~/.local
```

Or `packaging/build-deb.sh` for a `.deb`, `packaging/build-flatpak.sh` for a Flatpak.

## Configure

Sleeve writes an annotated `~/.config/sleeve/config.toml` on first run, and works
fine if you never open it. Worth setting:

```toml
locale   = "GB"               # which storefront to ask; decides prices and region-locking
currency = "GBP"
contact  = "you@example.com"  # optional — so MusicBrainz contacts you, not the project
```

The `[keys]` section is entirely optional; see the table above for what each one
adds.

Every scoring weight is in the same file, with the defaults written out and commented. Change one and re-run `./test.sh` — the ranking properties are asserted, so if an override breaks a principle the suite says which.

## Building

```sh
./test.sh              # fmt, clippy -D warnings, tests. The gate.
./test.sh --headless   # same, under Xvfb

cargo run --example preview -- /tmp/preview       # render the real pages to PNGs
cargo run --example preview -- /tmp/preview dark
```

No test touches the network. Every source is a pair of pure functions — build a request, parse a body — and the fixtures under `tests/fixtures/` were recorded from the live APIs, including the awkward ones: Bandcamp returning four cover versions of a famous album, Qobuz refusing an unauthenticated call, the Dynamic Range Database rate-limiting on the first request of the day.

## Licence

GPL-3.0-or-later.
