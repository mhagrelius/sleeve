# Sleeve — design for review

## Scope

An album buying guide for the GNOME desktop. Search a record, pick the pressing,
and get a ranked answer to "where should this money go", with the reasoning
attached.

The framing matters more than it sounds. A dozen applications answer "where can
I hear this". Sleeve answers "what should I buy", and those give different
answers to the same query — the second one has to weigh a file you keep against
access you rent, and what reaches the artist against what reaches a platform.
Nothing here recommends a stream when a purchase exists.

## The ranking

Four principles, in strict priority order:

1. Owning DRM-free files beats renting.
2. Higher artist payout beats lower.
3. Master quality beats format specs.
4. Actually available beats theoretically ideal.

### Why the sort is lexicographic

The scoring table alone does not hold these up. Worked from the brief's own
numbers: a tier-B iTunes purchase of a brickwalled master scores `50 − 5 = 45`,
and a tier-C Qobuz lossless stream of a good master scores
`30 + 4 + 5 + 2 + 8 = 49`. Under a plain numeric sort the stream wins, and
principle 1 — the one stated as "always" — quietly stops being true.

Reweighting fixes that instance and not the class. The weights are user-editable
by design, so any margin chosen today is one a person can erase tomorrow without
being told they have.

So the list sorts on `(tier, score)`. Tier is the primary key; score orders
within a tier. Principles 1 and 2 then hold by construction, principle 3's
"tiebreakers only" becomes literally true rather than a hope about magnitudes,
and no override can invert either. The numbers from the brief are unchanged and
still displayed — only the comparison changed.

`ordering = "numeric"` is available for anyone who wants the table read
literally. `score.rs` has a test that asserts the inversion it permits, so the
choice is informed rather than surprising.

### Region-locking is not a penalty

The brief scored it at −10. That says "slightly worse"; the truth is "you cannot
buy this". A −10 nudge also has an odd consequence — it makes a region-locked
purchase lose to a stream, which is arguably right for the wrong reason.

Region-locked offers leave the ranking and appear in their own section, score
intact. Principle 4 done properly: rank what exists *for you*.

### Where the brief's table was incomplete

- **Physical.** The payout bands did not cover it. A used disc pays the artist
  nothing, so it scores `0` and carries a caveat saying so; it is still tier A,
  because you end up owning lossless files. A new disc pays a label and gets
  `+3`.
- **Spotify.** Placed in tier D as lossy. Spotify Premium began serving
  24-bit/44.1 kHz FLAC in September 2025 and reached 50-plus markets by that
  October, so by the table's own criterion — tier is lossless vs lossy, payout
  is a separate modifier — it belongs in C. It is configuration rather than a
  constant, because it depends on the locale and on having Premium. Its payout
  stays at `0`, so it still sorts below Qobuz and Tidal inside the tier.
- **Tier E.** Not a rank. An album nobody sells is a statement about the album,
  so it lives on the verdict rather than occupying position N in a list, and it
  links nowhere.

## Sources

### The seam

Every source is two pure functions: one builds a `Request`, one parses a body
into an `Outcome`. Nothing under `model/` opens a socket; `ui/http.rs` does, and
it is the only file that does.

That is what makes the test suite offline and complete. Every source, every
malformed response, every rate-limit and every ranking is exercised from a body
recorded off the live API. A test needing a socket would mean the seam had
broken, not that the test was unlucky.

There is no `Source` trait. The eight sources answer genuinely different
questions — MusicBrainz "what is this record", Discogs "what pressings exist",
Bandcamp "will they actually sell it to me" — and a trait returning one uniform
type would be a fiction costing a conversion at both ends. What they share is
the request/parse shape, and the two types express that.

### Why `Outcome` has four variants

```rust
enum Outcome<T> { Found(T), Empty, Unusable(Reason), Stale(Reason) }
```

`Empty` and `Stale` are the pair that matter. Half these sources are
undocumented JSON endpoints and one is an HTML table; those do not fail with a
status code, they answer `200 OK` with a page that no longer contains what we
parse. If "this shop does not stock it" and "our parser broke" are the same
value, a broken Bandcamp reader silently deletes the best offer from every
ranking and the result still looks like a confident, complete answer.

`Stale` renders as "check failed", and it is what a fixture test asserts on when
a recorded body is refreshed.

### What the live APIs actually did

Four of the eight sources behaved differently from how the brief described them,
and each changed the design:

- **Bandcamp** needs no scraping. Two undocumented JSON endpoints their own
  mobile client uses — `fuzzysearch` and `tralbum_details` — answer with a
  price, a cover, and an `is_purchasable` flag, with no key and no browser
  impersonation. Two quirks: the search returns a **doubled URL**
  (`https://x.bandcamp.comhttps://x.bandcamp.com/album/y`), and being on
  Bandcamp is not being for sale — *Kid A* returns a real price and
  `is_purchasable: false`. Reading the price and skipping the flag would invent
  the top-ranked offer in the list.
- **Bandcamp's search is loose enough to be dangerous.** "Radiohead Kid A"
  returns, in order: a Halifax Music Co-op page titled "Radiohead - Kid A", a
  string quartet recital, two chiptune arrangements, and only then Radiohead's
  own listing. Four of five have "Radiohead" in the *title*. Matching on title
  alone buys a cover version, so artist and title must clear their floors
  independently — an average lets a perfect title carry a wrong artist. The cost
  is a false negative on label pages, which is the right way to be wrong.
- **Qobuz needs an account.** The catalogue answers `401 User authentication is
  required` to an `app_id` alone; it wants an `X-User-Auth-Token` from a
  logged-in session too. The refusal is a recorded fixture, and it must read as
  "you have not configured this" rather than "Qobuz is broken" — those send a
  person to different places. Sleeve never touches the signed endpoints that
  mint file URLs, so no `app_secret` is involved.
- **The Dynamic Range Database rate-limits immediately.** HTTP 429 on the first
  request of the day, with a polite User-Agent and with a browser one. It gets
  the longest backoff in the policy table and never sits on the critical path.

### The index that removes the credentials

The first cut of this needed a Discogs account and a paid Qobuz subscription
before it said anything much, and four boutique shops were simply unreachable
behind bot protection. Two findings undid all of that, and both were a `curl`
away:

**Discogs needs no token.** Search and release lookup both answer
unauthenticated, at 25 requests a minute against 60 with one. The brief said a
token was required and it was wrong.

**MusicBrainz indexes purchase links.** `inc=url-rels` on a release returns
`purchase for download` relations — for *Kid A*, pointing at Bleep, Qobuz,
Bandcamp and iTunes. Editors record where an album is sold, which means the
shops that refuse an HTTP client outright, and the one that wants a paid
subscription before its own API will answer, all arrive through a source already
being called, for free.

Those relations carry no price. That costs almost nothing here, because price is
reported and never scored — a tier-A Bleep row with an unknown price ranks
exactly where it belongs. It is the rare case where the scoring decision made
earlier pays for itself somewhere unrelated.

What it is not is a *check*. A link says somebody once saw the album for sale
there. Coverage depends on an editor having bothered, and it varies per
pressing rather than per album: the 2000 *Kid A* CD has no shop links at all
while the digital issue has five, so choosing a physical pressing also queries
its digital sibling. Offers from the index are marked `Provenance::Indexed`, and
`offer::merge` lets a live check beat them — **including a negative one**.
Bandcamp reports `is_purchasable: false` for records MusicBrainz still lists a
Bandcamp link for; without that rule the index would resurrect a tier-A offer
nobody can buy and put it top of the ranking.

So the credentials collapsed to two optional keys that only add information: a
Discogs token for a higher rate limit, and a Qobuz token for a price on a row
that already exists. **7digital** remains genuinely absent — a commercial
partner agreement, and no index can price it.

### Attribution is the hard part of DR, not fetching

The database is keyed by loose artist/album text and holds a row per pressing —
the 2000 CD, the 2009 remaster, a vinyl rip — with different values. Attaching
the wrong row moves a score by up to 13 points, which is worse than having no DR
at all.

So `best_match` refuses to guess. A row whose year matches the release wins; if
none does but every row agrees, the album has one master and the value is safe;
otherwise nothing is returned and the offer scores neutrally. Whatever is
matched is printed in the caveat line, so a misattribution is visible rather
than silent.

## Search

MusicBrainz relevance is the primary signal and a local pass re-ranks it,
because neither is enough alone: MusicBrainz scores a Lucene query and knows
nothing about a transposed letter, while a text pass knows nothing about which
of two identically titled records is the famous one.

The blend is `0.4 × relevance + 0.4 × title + 0.2 × artist`. The artist is a
third rather than half, because a search often has it slightly wrong — that is
the case the module exists to survive.

Two things had to be built rather than used. The Rust port of rapidfuzz ships
only `fuzz::ratio`, and it returns 0.0–1.0 where the Python library returns
0–100. And `token_set_ratio` alone is unusable here: by construction, when one
side's tokens are a subset of the other's it compares the shared tokens against
themselves and returns a perfect match, so "Kid A" scores 1.0 against "Kid A
Mnesia" and the two become indistinguishable. Half token-set and half plain
ratio keeps the tolerance for a missing subtitle while making extra words cost
something.

A title similarity below 0.6 vetoes a candidate outright, whatever MusicBrainz
thought. With relevance weighted at 0.4, a strong enough score clears the
blended floor on its own, and a plain textual disagreement has to be able to
override that.

**Confidence needs two conditions.** The leader must score above 0.80 *and* lead
the runner-up by 0.10. A high score alone is not enough: fifteen editions of one
symphony all match a search for that symphony, and picking the first is a coin
toss dressed as an answer. When it is ambiguous the page says so.

## The interface

Three pages in an `AdwNavigationView`: search, then which pressing, then where
to buy it. A drill-down rather than tabs, because the three are strictly
sequential and each one's content is chosen by the last.

```
┌─ Sleeve ──────────────────────────────────────────── ☰ ─┐
│  ┌──────────────────────────────────────────────────┐   │
│  │ Artist                                           │   │
│  │ Album                                            │   │
│  │                    Search                        │   │
│  └──────────────────────────────────────────────────┘   │
│                                                         │
│  Matches                                                │
│  Several of these fit equally well — pick the one you   │
│  meant                                                  │
│  ┌──────────────────────────────────────────────────┐   │
│  │ ▣  Blade Runner                        Vangelis ›│   │
│  │    1994 · Album · 1994 soundtrack album          │   │
│  │ ▣  Blade Runner                        Vangelis ›│   │
│  │    2007 · Album · Esper Edition, 3 discs         │   │
│  └──────────────────────────────────────────────────┘   │
│                                                         │
│  Also close                                             │
│  Records with similar names, kept apart from above      │
│  ┌──────────────────────────────────────────────────┐   │
│  │ ▣  Blade Runner       The New American Orchestra ›│  │
│  │    Different artist, similar title · 1982        │   │
│  └──────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────┘
```

Near misses have their own group, their own heading, and a per-row line saying
why each is there. Interleaving them would make a confident answer look like a
guess; hiding them would lose the thing that most often turns out to be what was
wanted.

The result page reads top to bottom in order of what matters: the recommendation
in plain words, then anything odd about the results, then the ranking, then what
could not be checked.

### Threading

No worker threads and no channels. libsoup's async calls complete on the GLib
main loop, so each source's callback fires when its answer lands and updates the
view then. The ranking is recomputed and redrawn on every arrival, so Bandcamp
appearing four seconds after iTunes reorders the list in front of you rather
than delaying all of it. A source that never answers leaves a row in "not
checked" and costs nothing else.

Rate limiting is a per-source token bucket, because the budgets differ by two
orders of magnitude — MusicBrainz publishes one request a second and *blocks*
rather than throttles; Odesli allows about ten a minute. A request arriving too
early is scheduled on a `glib::timeout`, never slept on.

A generation counter guards navigation: a callback from a lookup the person has
already left finds a stale generation and drops its result instead of writing it
into the page that replaced it.

### Cover art

Never waited for. A row renders with a dimmed symbolic placeholder immediately
and swaps in the picture whenever it arrives; if none ever arrives the
placeholder is the answer. Nothing here reports an error, because there is no
error a person could act on.

The fallback order is by how well a source lines up with what has been resolved,
not by picture quality: the Cover Art Archive is MBID-keyed, so it is the only
one that cannot be showing a different edition's sleeve. Then iTunes artwork —
already fetched for the price, and resized by rewriting the dimensions in the
URL — then Bandcamp's, then Deezer.

Files live under `~/.cache/sleeve/covers` with only their paths in SQLite. A
database full of JPEGs is one you cannot inspect or copy. **Misses are cached
too**, at a shorter TTL, so an album with no art anywhere stops re-running the
whole chain on every search.

## What this deliberately does not do

- **Suggest an unauthorised source.** An album with no legitimate purchase path
  is reported as exactly that, with a reason and no links.
- **Convert prices.** A converted price shown as a local one is a guess about an
  exchange rate presented as a fact about a shop. Each price appears in the
  currency its vendor quoted.
- **Automate a purchase.** It tells you where to go and opens the page.
- **Score price.** Reported, never ranked. When the best and cheapest differ,
  the recommendation says so and leaves the choice alone.
