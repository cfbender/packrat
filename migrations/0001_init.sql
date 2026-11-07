-- Catalog tables mirror the parts of MTGJSON's AllPrintings.sqlite that
-- pack simulation needs: sets, cards (identity and display only), booster
-- configurations (sheets with per-card weights and pack variants with
-- per-sheet pick counts), and sealed products (what you can buy, what it
-- contains, and the vendor ids its prices are keyed by).

CREATE TABLE meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE sets (
  code TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  release_date TEXT,
  set_type TEXT,
  parent_code TEXT,
  keyrune_code TEXT,
  tcgplayer_group_id INTEGER
);

CREATE TABLE cards (
  uuid TEXT PRIMARY KEY,
  scryfall_id TEXT,
  name TEXT NOT NULL,
  set_code TEXT NOT NULL,
  number TEXT NOT NULL,
  rarity TEXT,
  colors TEXT,
  types TEXT,
  layout TEXT,
  finishes TEXT,
  promo_types TEXT,
  frame_effects TEXT,
  border_color TEXT,
  tcgplayer_product_id INTEGER,
  card_kingdom_id INTEGER,
  card_kingdom_foil_id INTEGER
);
CREATE INDEX cards_set_code ON cards (set_code);

CREATE TABLE booster_sheets (
  set_code TEXT NOT NULL,
  booster_code TEXT NOT NULL,
  sheet TEXT NOT NULL,
  is_foil INTEGER NOT NULL DEFAULT 0,
  balance_colors INTEGER NOT NULL DEFAULT 0,
  total_weight INTEGER NOT NULL,
  PRIMARY KEY (set_code, booster_code, sheet)
);

CREATE TABLE booster_sheet_cards (
  set_code TEXT NOT NULL,
  booster_code TEXT NOT NULL,
  sheet TEXT NOT NULL,
  card_uuid TEXT NOT NULL,
  weight INTEGER NOT NULL
);
CREATE INDEX booster_sheet_cards_sheet
  ON booster_sheet_cards (set_code, booster_code, sheet);

CREATE TABLE booster_variants (
  set_code TEXT NOT NULL,
  booster_code TEXT NOT NULL,
  variant INTEGER NOT NULL,
  weight INTEGER NOT NULL,
  PRIMARY KEY (set_code, booster_code, variant)
);

CREATE TABLE booster_variant_slots (
  set_code TEXT NOT NULL,
  booster_code TEXT NOT NULL,
  variant INTEGER NOT NULL,
  sheet TEXT NOT NULL,
  picks INTEGER NOT NULL,
  PRIMARY KEY (set_code, booster_code, variant, sheet)
);

CREATE TABLE sealed_products (
  uuid TEXT PRIMARY KEY,
  set_code TEXT NOT NULL,
  name TEXT NOT NULL,
  category TEXT,
  subtype TEXT,
  card_count INTEGER,
  contents TEXT,
  tcgplayer_product_id INTEGER,
  card_kingdom_id INTEGER,
  release_date TEXT
);
CREATE INDEX sealed_products_set_code ON sealed_products (set_code);

-- Retail prices in cents; `finish` is nonfoil, foil, or etched.
CREATE TABLE card_prices (
  card_uuid TEXT NOT NULL,
  vendor TEXT NOT NULL,
  finish TEXT NOT NULL,
  cents INTEGER NOT NULL,
  PRIMARY KEY (card_uuid, vendor, finish)
);

CREATE TABLE sealed_prices (
  product_uuid TEXT NOT NULL,
  vendor TEXT NOT NULL,
  cents INTEGER NOT NULL,
  PRIMARY KEY (product_uuid, vendor)
);
