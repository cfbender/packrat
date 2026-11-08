//! The in-memory catalog snapshot: booster configurations, sealed products,
//! and prices, loaded from SQLite after each import. Simulation and EV read
//! from it without touching the database; card display details (name,
//! image, rarity) stay in SQLite and are fetched per opened pack.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Vendor {
    TcgPlayer,
    CardKingdom,
    ManaPool,
}

impl Vendor {
    pub const ALL: [Vendor; 3] = [Vendor::TcgPlayer, Vendor::CardKingdom, Vendor::ManaPool];

    pub fn as_str(self) -> &'static str {
        match self {
            Vendor::TcgPlayer => "tcgplayer",
            Vendor::CardKingdom => "cardkingdom",
            Vendor::ManaPool => "manapool",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "tcgplayer" => Some(Vendor::TcgPlayer),
            "cardkingdom" => Some(Vendor::CardKingdom),
            "manapool" => Some(Vendor::ManaPool),
            _ => None,
        }
    }
}

/// Which vendor's prices to use and the floor below which a card counts
/// as $0 (bulk is rarely sellable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pricing {
    pub vendor: Vendor,
    pub floor_cents: u32,
}

impl Pricing {
    pub fn new(vendor: Vendor) -> Self {
        Pricing {
            vendor,
            floor_cents: 0,
        }
    }

    pub fn with_floor(self, floor_cents: u32) -> Self {
        Pricing {
            floor_cents,
            ..self
        }
    }
}

/// Color bits for color balancing: W U B R G.
pub const COLOR_BITS: [(char, u8); 5] = [('W', 1), ('U', 2), ('B', 4), ('R', 8), ('G', 16)];

pub fn color_mask(colors: &str) -> u8 {
    colors
        .split(',')
        .map(str::trim)
        .filter_map(|c| c.chars().next())
        .filter_map(|c| COLOR_BITS.iter().find(|(k, _)| *k == c).map(|(_, b)| *b))
        .fold(0, |acc, b| acc | b)
}

#[derive(Debug, Clone)]
pub struct SheetCard {
    pub uuid: Arc<str>,
    pub weight: u64,
    pub colors: u8,
}

#[derive(Debug, Clone)]
pub struct Sheet {
    pub name: String,
    pub foil: bool,
    pub balance_colors: bool,
    pub total_weight: u64,
    pub cards: Vec<SheetCard>,
}

#[derive(Debug, Clone, Copy)]
pub struct Slot {
    pub sheet: usize,
    pub picks: u32,
}

#[derive(Debug, Clone)]
pub struct Variant {
    pub weight: u64,
    pub slots: Vec<Slot>,
}

#[derive(Debug, Clone)]
pub struct Booster {
    pub set_code: String,
    pub code: String,
    pub sheets: Vec<Sheet>,
    pub variants: Vec<Variant>,
    pub total_weight: u64,
}

impl Booster {
    /// Average number of cards in a pack of this configuration.
    pub fn card_count(&self) -> f64 {
        if self.total_weight == 0 {
            return 0.0;
        }
        self.variants
            .iter()
            .map(|v| v.weight as f64 * v.slots.iter().map(|s| s.picks as f64).sum::<f64>())
            .sum::<f64>()
            / self.total_weight as f64
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct FinishPrices {
    pub nonfoil: Option<u32>,
    pub foil: Option<u32>,
    pub etched: Option<u32>,
}

impl FinishPrices {
    /// Price for a card pulled from a foil or nonfoil sheet, falling back
    /// across finishes when the vendor lists only one.
    pub fn for_sheet(&self, foil: bool) -> Option<u32> {
        if foil {
            self.foil.or(self.etched).or(self.nonfoil)
        } else {
            self.nonfoil.or(self.foil).or(self.etched)
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PackRef {
    pub set: String,
    pub code: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SealedRef {
    pub uuid: String,
    pub count: u32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CardRef {
    pub uuid: String,
    pub foil: bool,
}

/// What a sealed product contains, reduced to what EV can price. `priced`
/// is false when MTGJSON lists contents we cannot value (decks, variable
/// configurations).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Contents {
    pub packs: Vec<PackRef>,
    pub sealed: Vec<SealedRef>,
    pub cards: Vec<CardRef>,
    pub decks: usize,
    pub variable: bool,
}

impl Contents {
    pub fn parse(json: Option<&str>) -> Self {
        let Some(json) = json else {
            return Self::default();
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
            return Self::default();
        };
        let mut contents = Self::default();
        if let Some(packs) = value.get("pack").and_then(|v| v.as_array()) {
            for p in packs {
                if let (Some(set), Some(code)) = (
                    p.get("set").and_then(|v| v.as_str()),
                    p.get("code").and_then(|v| v.as_str()),
                ) {
                    contents.packs.push(PackRef {
                        set: set.to_ascii_uppercase(),
                        code: code.to_owned(),
                    });
                }
            }
        }
        if let Some(sealed) = value.get("sealed").and_then(|v| v.as_array()) {
            for s in sealed {
                if let Some(uuid) = s.get("uuid").and_then(|v| v.as_str()) {
                    contents.sealed.push(SealedRef {
                        uuid: uuid.to_owned(),
                        count: s.get("count").and_then(|v| v.as_u64()).unwrap_or(1) as u32,
                    });
                }
            }
        }
        if let Some(cards) = value.get("card").and_then(|v| v.as_array()) {
            for c in cards {
                if let Some(uuid) = c.get("uuid").and_then(|v| v.as_str()) {
                    contents.cards.push(CardRef {
                        uuid: uuid.to_owned(),
                        foil: c.get("foil").and_then(|v| v.as_bool()).unwrap_or(false),
                    });
                }
            }
        }
        contents.decks = value
            .get("deck")
            .and_then(|v| v.as_array())
            .map_or(0, Vec::len);
        contents.variable = value.get("variable").is_some();
        contents
    }

    pub fn is_empty(&self) -> bool {
        self.packs.is_empty() && self.sealed.is_empty() && self.cards.is_empty()
    }

    /// The packs a booster box is made of: its most numerous sealed entry.
    /// Many boxes also bundle a single box-topper pack, which this skips.
    pub fn box_packs(&self) -> Option<&SealedRef> {
        self.sealed.iter().max_by_key(|s| s.count).filter(|s| s.count >= 2)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Product {
    pub uuid: String,
    pub set_code: String,
    pub name: String,
    pub category: Option<String>,
    pub subtype: Option<String>,
    pub card_count: Option<i64>,
    pub release_date: Option<String>,
    pub tcgplayer_product_id: Option<i64>,
    pub card_kingdom_id: Option<i64>,
    #[serde(skip)]
    pub contents: Contents,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetInfo {
    pub code: String,
    pub name: String,
    pub release_date: Option<String>,
    pub set_type: Option<String>,
    pub keyrune_code: Option<String>,
}

#[derive(Debug, Default)]
pub struct Catalog {
    pub sets: HashMap<String, SetInfo>,
    pub boosters: HashMap<(String, String), Arc<Booster>>,
    pub products: HashMap<String, Product>,
    pub card_prices: HashMap<Vendor, HashMap<Arc<str>, FinishPrices>>,
    pub sealed_prices: HashMap<Vendor, HashMap<String, u32>>,
    pub pack_by_booster: HashMap<(String, String), String>,
    pub box_by_pack: HashMap<String, String>,
    pub mtgjson_version: Option<String>,
    pub prices_updated_at: Option<String>,
}

impl Catalog {
    pub fn booster(&self, set_code: &str, code: &str) -> Option<Arc<Booster>> {
        self.boosters
            .get(&(set_code.to_ascii_uppercase(), code.to_owned()))
            .cloned()
    }

    /// The booster a sealed product opens into: its first pack, or the first
    /// pack of the most numerous sealed product nested inside it (a box of
    /// packs, skipping a bundled box topper).
    pub fn primary_booster(&self, uuid: &str) -> Option<Arc<Booster>> {
        let mut uuid = uuid;
        for _ in 0..6 {
            let product = self.products.get(uuid)?;
            if let Some(pack) = product.contents.packs.first() {
                return self.booster(&pack.set, &pack.code);
            }
            uuid = &product.contents.sealed.iter().max_by_key(|s| s.count)?.uuid;
        }
        None
    }

    pub fn card_price(&self, vendor: Vendor, uuid: &str, foil: bool) -> Option<u32> {
        self.card_prices
            .get(&vendor)?
            .get(uuid)
            .and_then(|p| p.for_sheet(foil))
    }

    /// Price under `pricing`: `None` when unknown, `Some(0)` when below the floor.
    pub fn priced(&self, pricing: Pricing, uuid: &str, foil: bool) -> Option<u32> {
        self.card_price(pricing.vendor, uuid, foil).map(|cents| {
            if cents < pricing.floor_cents {
                0
            } else {
                cents
            }
        })
    }

    pub fn card_prices_all(&self, uuid: &str, foil: bool) -> HashMap<Vendor, u32> {
        Vendor::ALL
            .into_iter()
            .filter_map(|v| self.card_price(v, uuid, foil).map(|p| (v, p)))
            .collect()
    }

    pub fn sealed_price(&self, vendor: Vendor, uuid: &str) -> Option<u32> {
        self.sealed_prices.get(&vendor)?.get(uuid).copied()
    }

    pub fn sealed_prices_all(&self, uuid: &str) -> HashMap<Vendor, u32> {
        Vendor::ALL
            .into_iter()
            .filter_map(|v| self.sealed_price(v, uuid).map(|p| (v, p)))
            .collect()
    }

    /// The booster-pack product whose contents are exactly this booster.
    pub fn pack_product(&self, set_code: &str, code: &str) -> Option<&Product> {
        let uuid = self
            .pack_by_booster
            .get(&(set_code.to_owned(), code.to_owned()))?;
        self.products.get(uuid)
    }

    /// The box made of N copies of this pack product (plus, sometimes, a box
    /// topper), with how many packs it holds.
    pub fn box_product(&self, pack_uuid: &str) -> Option<(&Product, u32)> {
        let uuid = self.box_by_pack.get(pack_uuid)?;
        let product = self.products.get(uuid)?;
        Some((product, product.contents.box_packs()?.count))
    }

    /// Everything a box opens into, as (booster, pack product name, packs):
    /// the main packs first, then any bundled extras such as a box topper.
    /// Sealed entries that are not a single booster pack are skipped.
    pub fn box_contents(&self, box_uuid: &str) -> Vec<(Arc<Booster>, String, u32)> {
        let Some(product) = self.products.get(box_uuid) else {
            return Vec::new();
        };
        let main = product.contents.box_packs().map(|s| s.uuid.as_str());
        let mut entries: Vec<&SealedRef> = product.contents.sealed.iter().collect();
        entries.sort_by_key(|s| Some(s.uuid.as_str()) != main);
        entries
            .into_iter()
            .filter_map(|entry| {
                let pack = self.products.get(&entry.uuid)?;
                let c = &pack.contents;
                if c.packs.len() != 1 || !c.sealed.is_empty() || !c.cards.is_empty() {
                    return None;
                }
                let booster = self.booster(&c.packs[0].set, &c.packs[0].code)?;
                Some((booster, pack.name.clone(), entry.count))
            })
            .collect()
    }

    /// Builds the booster -> pack product and pack -> box product indexes.
    fn index_products(&mut self) {
        let mut pack_candidates: HashMap<(String, String), Vec<&Product>> = HashMap::new();
        let mut box_candidates: HashMap<String, Vec<&Product>> = HashMap::new();
        for p in self.products.values() {
            let c = &p.contents;
            if c.packs.len() == 1 && c.sealed.is_empty() && c.cards.is_empty() {
                pack_candidates
                    .entry((c.packs[0].set.clone(), c.packs[0].code.clone()))
                    .or_default()
                    .push(p);
            }
            if c.packs.is_empty()
                && c.cards.is_empty()
                && p.category.as_deref() == Some("booster_box")
                && let Some(packs) = c.box_packs()
            {
                box_candidates.entry(packs.uuid.clone()).or_default().push(p);
            }
        }
        let pack_by_booster: HashMap<(String, String), String> = pack_candidates
            .into_iter()
            .filter_map(|(key, candidates)| {
                candidates
                    .into_iter()
                    .min_by_key(|p| (p.category.as_deref() != Some("booster_pack"), p.name.len()))
                    .map(|p| (key, p.uuid.clone()))
            })
            .collect();
        let box_by_pack: HashMap<String, String> = box_candidates
            .into_iter()
            .filter_map(|(key, candidates)| {
                candidates
                    .into_iter()
                    .max_by_key(|p| p.contents.box_packs().map_or(0, |s| s.count))
                    .map(|p| (key, p.uuid.clone()))
            })
            .collect();
        self.pack_by_booster = pack_by_booster;
        self.box_by_pack = box_by_pack;
    }

    pub async fn load(pool: &SqlitePool) -> anyhow::Result<Self> {
        let mut catalog = Catalog {
            mtgjson_version: crate::db::meta_get(pool, "mtgjson_version").await?,
            prices_updated_at: crate::db::meta_get(pool, "prices_updated_at").await?,
            ..Default::default()
        };

        for row in sqlx::query("SELECT code, name, release_date, set_type, keyrune_code FROM sets")
            .fetch_all(pool)
            .await?
        {
            let code: String = row.get(0);
            catalog.sets.insert(
                code.clone(),
                SetInfo {
                    code,
                    name: row.get(1),
                    release_date: row.get(2),
                    set_type: row.get(3),
                    keyrune_code: row.get(4),
                },
            );
        }

        // Sheets, with their cards (and colors for balancing).
        let mut sheets: HashMap<(String, String), Vec<Sheet>> = HashMap::new();
        let mut sheet_index: HashMap<(String, String, String), usize> = HashMap::new();
        for row in sqlx::query(
            "SELECT set_code, booster_code, sheet, is_foil, balance_colors, total_weight
             FROM booster_sheets ORDER BY set_code, booster_code, sheet",
        )
        .fetch_all(pool)
        .await?
        {
            let set_code: String = row.get(0);
            let booster_code: String = row.get(1);
            let name: String = row.get(2);
            let list = sheets
                .entry((set_code.clone(), booster_code.clone()))
                .or_default();
            sheet_index.insert((set_code, booster_code, name.clone()), list.len());
            list.push(Sheet {
                name,
                foil: row.get::<i64, _>(3) != 0,
                balance_colors: row.get::<i64, _>(4) != 0,
                total_weight: row.get::<i64, _>(5).max(0) as u64,
                cards: Vec::new(),
            });
        }

        let mut uuids: HashMap<String, Arc<str>> = HashMap::new();
        let mut intern = |s: String| -> Arc<str> {
            uuids
                .entry(s.clone())
                .or_insert_with(|| Arc::from(s.as_str()))
                .clone()
        };
        for row in sqlx::query(
            "SELECT sc.set_code, sc.booster_code, sc.sheet, sc.card_uuid, sc.weight, COALESCE(c.colors, '')
             FROM booster_sheet_cards sc LEFT JOIN cards c ON c.uuid = sc.card_uuid",
        )
        .fetch_all(pool)
        .await?
        {
            let key = (row.get::<String, _>(0), row.get::<String, _>(1), row.get::<String, _>(2));
            let Some(&index) = sheet_index.get(&key) else {
                continue;
            };
            let uuid = intern(row.get::<String, _>(3));
            let colors: String = row.get(5);
            if let Some(list) = sheets.get_mut(&(key.0, key.1)) {
                list[index].cards.push(SheetCard {
                    uuid,
                    weight: row.get::<i64, _>(4).max(0) as u64,
                    colors: color_mask(&colors),
                });
            }
        }
        // Trust the cards over the recorded total when they disagree.
        for list in sheets.values_mut() {
            for sheet in list {
                let sum: u64 = sheet.cards.iter().map(|c| c.weight).sum();
                if sum > 0 {
                    sheet.total_weight = sum;
                }
            }
        }

        let mut variants: HashMap<(String, String), Vec<Variant>> = HashMap::new();
        for row in sqlx::query(
            "SELECT set_code, booster_code, variant, weight FROM booster_variants
             ORDER BY set_code, booster_code, variant",
        )
        .fetch_all(pool)
        .await?
        {
            let key = (row.get::<String, _>(0), row.get::<String, _>(1));
            let variant: i64 = row.get(2);
            let list = variants.entry(key).or_default();
            while list.len() <= variant as usize {
                list.push(Variant {
                    weight: 0,
                    slots: Vec::new(),
                });
            }
            list[variant as usize].weight = row.get::<i64, _>(3).max(0) as u64;
        }
        for row in sqlx::query(
            "SELECT set_code, booster_code, variant, sheet, picks FROM booster_variant_slots",
        )
        .fetch_all(pool)
        .await?
        {
            let set_code: String = row.get(0);
            let booster_code: String = row.get(1);
            let variant: i64 = row.get(2);
            let sheet: String = row.get(3);
            let Some(&index) = sheet_index.get(&(set_code.clone(), booster_code.clone(), sheet))
            else {
                continue;
            };
            if let Some(list) = variants.get_mut(&(set_code, booster_code))
                && let Some(v) = list.get_mut(variant as usize)
            {
                v.slots.push(Slot {
                    sheet: index,
                    picks: row.get::<i64, _>(4).max(0) as u32,
                });
            }
        }

        for ((set_code, code), sheets) in sheets {
            let variants = variants
                .remove(&(set_code.clone(), code.clone()))
                .unwrap_or_default();
            let variants: Vec<Variant> = variants
                .into_iter()
                .filter(|v| v.weight > 0 && !v.slots.is_empty())
                .collect();
            if variants.is_empty() || sheets.iter().all(|s| s.cards.is_empty()) {
                continue;
            }
            let total_weight = variants.iter().map(|v| v.weight).sum();
            catalog.boosters.insert(
                (set_code.clone(), code.clone()),
                Arc::new(Booster {
                    set_code,
                    code,
                    sheets,
                    variants,
                    total_weight,
                }),
            );
        }

        for row in sqlx::query(
            "SELECT uuid, set_code, name, category, subtype, card_count, contents,
                    tcgplayer_product_id, card_kingdom_id, release_date
             FROM sealed_products",
        )
        .fetch_all(pool)
        .await?
        {
            let uuid: String = row.get(0);
            let contents: Option<String> = row.get(6);
            catalog.products.insert(
                uuid.clone(),
                Product {
                    uuid,
                    set_code: row.get(1),
                    name: row.get(2),
                    category: row.get(3),
                    subtype: row.get(4),
                    card_count: row.get(5),
                    contents: Contents::parse(contents.as_deref()),
                    tcgplayer_product_id: row.get(7),
                    card_kingdom_id: row.get(8),
                    release_date: row.get(9),
                },
            );
        }

        catalog.index_products();

        for row in sqlx::query("SELECT card_uuid, vendor, finish, cents FROM card_prices")
            .fetch_all(pool)
            .await?
        {
            let Some(vendor) = Vendor::parse(row.get::<String, _>(1).as_str()) else {
                continue;
            };
            let uuid = intern(row.get::<String, _>(0));
            let cents = row.get::<i64, _>(3).clamp(0, u32::MAX as i64) as u32;
            let entry = catalog
                .card_prices
                .entry(vendor)
                .or_default()
                .entry(uuid)
                .or_default();
            match row.get::<String, _>(2).as_str() {
                "foil" => entry.foil = Some(cents),
                "etched" => entry.etched = Some(cents),
                _ => entry.nonfoil = Some(cents),
            }
        }

        for row in sqlx::query("SELECT product_uuid, vendor, cents FROM sealed_prices")
            .fetch_all(pool)
            .await?
        {
            let Some(vendor) = Vendor::parse(row.get::<String, _>(1).as_str()) else {
                continue;
            };
            let cents = row.get::<i64, _>(2).clamp(0, u32::MAX as i64) as u32;
            catalog
                .sealed_prices
                .entry(vendor)
                .or_default()
                .insert(row.get(0), cents);
        }

        Ok(catalog)
    }
}
