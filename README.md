# packrat

Self-hosted Magic: The Gathering booster pack simulator with expected-value
statistics for sealed product. Open packs, see what they're worth, and rank
every sealed product by EV against its current price.

- Rust (axum + SQLite) backend, vanilla TypeScript + Vite PWA frontend.
- Pack contents come from [MTGJSON](https://mtgjson.com) booster sheets, so
  pull rates match the published odds for every set MTGJSON covers.
- Single prices from MTGJSON `AllPricesToday` (TCGplayer market, Card Kingdom,
  ManaPool). Sealed prices from ManaPool, Card Kingdom, and TCGplayer (via
  [tcgcsv](https://tcgcsv.com)). Card images from Scryfall.
- Everything is cached locally; the only network use is the periodic price
  refresh and card images.

## Running

### Docker

```sh
docker compose up -d
```

Then open <http://localhost:4100>. The first start downloads the MTGJSON
database (~130 MB compressed) and current prices into the `packrat-data`
volume; this takes a few minutes and only happens once. Prices refresh every
`PACKRAT_PRICE_REFRESH_HOURS` (default 12) while the server runs, and the
catalog is re-imported when MTGJSON publishes a new build.

### From source

Requires Rust (stable) and Node 22 + pnpm.

```sh
(cd web && pnpm install && pnpm run build)
cargo run --release -- serve
```

Other commands: `packrat import [--force]` (catalog + prices, then exit) and
`packrat prices` (prices only).

For frontend development, `pnpm run dev` in `web/` serves the app on port
5174 and proxies `/api` to the Rust server on `$PORT` (default 4100).

### Configuration

| Variable                      | Default    | Purpose                              |
| ----------------------------- | ---------- | ------------------------------------ |
| `PORT`                        | `4100`     | HTTP port                            |
| `PACKRAT_DATA_DIR`            | `data`     | SQLite database and download cache   |
| `PACKRAT_STATIC_DIR`          | `web/dist` | Built web app to serve               |
| `PACKRAT_PRICE_REFRESH_HOURS` | `12`       | Price refresh interval               |
| `PACKRAT_MTGJSON_URL`         | mtgjson.com | Alternate MTGJSON mirror            |

## How the numbers are computed

A booster in MTGJSON is a weighted set of *variants* (pack configurations),
each a list of slots that pick N cards from a weighted *sheet*. The simulator
draws a variant by weight, then draws each slot's cards from its sheet without
replacement, applying MTGJSON's colour-balancing flag where set.

**Expected value** is closed-form: for each sheet, the mean and variance of one
pick are computed from the card weights and prices; slots add these up, and
the mixture over variants gives the pack's mean and variance. Picks within a
slot are treated as independent (a tight approximation for sheets much larger
than the pick count). Sealed products are valued recursively from the packs
they contain; products with decks or variable contents are skipped.

**Simulated** statistics (median, min/max, break-even share, histogram) come
from 1,000 simulated openings. Verdict tiers:
positive EV ≥ 115% of price, near break-even 95–115%, fair 80–95%, poor
60–80%, very poor below; "lottery ticket" when at most 15% of openings beat
the price and the standard deviation is at least 0.75× the price.

Cards without a price for the chosen vendor count as $0. The price-floor
selector ("Ignore cards under $X") zeroes cards below the floor, which is the
usual way to discount bulk you would never actually sell.

## API

| Endpoint        | Description                                                       |
| --------------- | ----------------------------------------------------------------- |
| `GET /api/status`   | Catalog version, price age, background job state                |
| `GET /api/products` | Every booster with its pack/box product and sealed prices       |
| `POST /api/open`    | `{set_code, booster_code, count, floor}` → opened packs          |
| `GET /api/stats`    | `?set_code&booster_code&vendor&floor&samples` → EV and sim stats |
| `GET /api/ev`       | `?vendor&floor` → every priced sealed product ranked by EV/price |
| `POST /api/refresh` | `{catalog?: bool}` → kick off a price (or full) refresh          |

Vendors: `tcgplayer`, `cardkingdom`, `manapool`. Money in the API is cents for
prices on cards and products, dollars for statistics.
