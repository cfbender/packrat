//! Price feeds. Card prices come from MTGJSON's `AllPricesToday.json.gz`
//! (one small file with TCGplayer, Card Kingdom, and ManaPool retail prices
//! keyed by card uuid). MTGJSON carries no sealed prices, so those come from
//! each vendor's own public feed, joined to sealed products through the
//! vendor ids MTGJSON records on them.

use std::collections::HashMap;
use std::io::Read;
use std::time::Duration;

use anyhow::Context;
use serde::Deserialize;
use sqlx::{QueryBuilder, Row, SqlitePool};

use crate::catalog::Vendor;
use crate::db;
use crate::mtgjson::Progress;

pub const MANAPOOL_SEALED_URL: &str = "https://manapool.com/api/v1/prices/sealed";
pub const CARD_KINGDOM_SEALED_URL: &str = "https://api.cardkingdom.com/api/sealed_pricelist";
pub const TCGCSV_BASE_URL: &str = "https://tcgcsv.com/tcgplayer/1";
/// tcgcsv asks scrapers to pace requests.
const TCGCSV_DELAY: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CardPriceRow<'a> {
    pub uuid: &'a str,
    pub vendor: Vendor,
    pub finish: &'static str,
    pub cents: u32,
}

/// Refreshes every feed. Feed failures are logged and skipped so one vendor
/// outage never blocks the others.
pub async fn refresh_all(
    pool: &SqlitePool,
    client: &reqwest::Client,
    mtgjson_base_url: &str,
    progress: Progress,
) -> anyhow::Result<()> {
    progress("Downloading card prices (MTGJSON AllPricesToday)".to_owned());
    match import_card_prices(pool, client, mtgjson_base_url).await {
        Ok(rows) => progress(format!("Imported {rows} card prices")),
        Err(error) => tracing::warn!(%error, "card price import failed"),
    }

    progress("Downloading ManaPool sealed prices".to_owned());
    match import_manapool_sealed(pool, client).await {
        Ok(rows) => progress(format!("Imported {rows} ManaPool sealed prices")),
        Err(error) => tracing::warn!(%error, "ManaPool sealed import failed"),
    }

    progress("Downloading Card Kingdom sealed prices".to_owned());
    match import_card_kingdom_sealed(pool, client).await {
        Ok(rows) => progress(format!("Imported {rows} Card Kingdom sealed prices")),
        Err(error) => tracing::warn!(%error, "Card Kingdom sealed import failed"),
    }

    progress("Downloading TCGplayer sealed prices (tcgcsv)".to_owned());
    match import_tcgcsv_sealed(pool, client, progress.clone()).await {
        Ok(rows) => progress(format!("Imported {rows} TCGplayer sealed prices")),
        Err(error) => tracing::warn!(%error, "tcgcsv sealed import failed"),
    }

    db::meta_set(pool, "prices_updated_at", &db::now()).await?;
    Ok(())
}

#[derive(Deserialize)]
struct PriceFile {
    data: HashMap<String, PriceEntry>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PriceEntry {
    paper: HashMap<String, VendorPrices>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct VendorPrices {
    /// finish -> date -> price
    retail: HashMap<String, HashMap<String, f64>>,
    currency: String,
}

fn latest(prices: &HashMap<String, f64>) -> Option<f64> {
    prices
        .iter()
        .max_by(|a, b| a.0.cmp(b.0))
        .map(|(_, price)| *price)
}

pub fn cents(price: f64) -> Option<u32> {
    if !price.is_finite() || price < 0.0 {
        return None;
    }
    Some((price * 100.0).round() as u32)
}

/// Card price rows from a decoded `AllPricesToday` file, USD vendors only.
fn card_price_rows(file: &PriceFile) -> Vec<CardPriceRow<'_>> {
    let mut rows = Vec::new();
    for (uuid, entry) in &file.data {
        for (vendor_name, prices) in &entry.paper {
            let Some(vendor) = Vendor::parse(vendor_name) else {
                continue;
            };
            if prices.currency != "USD" {
                continue;
            }
            for (finish, by_date) in &prices.retail {
                let finish = match finish.as_str() {
                    "normal" => "nonfoil",
                    "foil" => "foil",
                    "etched" => "etched",
                    _ => continue,
                };
                if let Some(cents) = latest(by_date).and_then(cents) {
                    rows.push(CardPriceRow {
                        uuid,
                        vendor,
                        finish,
                        cents,
                    });
                }
            }
        }
    }
    rows
}

pub async fn import_card_prices(
    pool: &SqlitePool,
    client: &reqwest::Client,
    base_url: &str,
) -> anyhow::Result<usize> {
    let body =
        crate::http::get_bytes(client, &format!("{base_url}/AllPricesToday.json.gz")).await?;
    let file = tokio::task::spawn_blocking(move || -> anyhow::Result<PriceFile> {
        let mut decoder = flate2::read::GzDecoder::new(&body[..]);
        let mut json = Vec::with_capacity(body.len() * 8);
        decoder
            .read_to_end(&mut json)
            .context("gunzip AllPricesToday")?;
        serde_json::from_slice(&json).context("parse AllPricesToday")
    })
    .await??;
    let rows = card_price_rows(&file);
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM card_prices")
        .execute(&mut *tx)
        .await?;
    for chunk in rows.chunks(500) {
        let mut builder: QueryBuilder<sqlx::Sqlite> = QueryBuilder::new(
            "INSERT OR REPLACE INTO card_prices (card_uuid, vendor, finish, cents) ",
        );
        builder.push_values(chunk, |mut b, row| {
            b.push_bind(row.uuid)
                .push_bind(row.vendor.as_str())
                .push_bind(row.finish)
                .push_bind(row.cents as i64);
        });
        builder.build().execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(rows.len())
}

async fn replace_sealed_prices(
    pool: &SqlitePool,
    vendor: Vendor,
    rows: &[(String, u32)],
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM sealed_prices WHERE vendor = ?")
        .bind(vendor.as_str())
        .execute(&mut *tx)
        .await?;
    for chunk in rows.chunks(500) {
        let mut builder: QueryBuilder<sqlx::Sqlite> = QueryBuilder::new(
            "INSERT OR REPLACE INTO sealed_prices (product_uuid, vendor, cents) ",
        );
        builder.push_values(chunk, |mut b, (uuid, cents)| {
            b.push_bind(uuid)
                .push_bind(vendor.as_str())
                .push_bind(*cents as i64);
        });
        builder.build().execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

#[derive(Clone, Copy)]
enum VendorId {
    TcgPlayer,
    CardKingdom,
}

/// Sealed product uuids by a vendor id.
async fn products_by_id(
    pool: &SqlitePool,
    id: VendorId,
) -> anyhow::Result<HashMap<i64, Vec<String>>> {
    let query = match id {
        VendorId::TcgPlayer => sqlx::query(
            "SELECT tcgplayer_product_id, uuid FROM sealed_products WHERE tcgplayer_product_id IS NOT NULL",
        ),
        VendorId::CardKingdom => sqlx::query(
            "SELECT card_kingdom_id, uuid FROM sealed_products WHERE card_kingdom_id IS NOT NULL",
        ),
    };
    let mut map: HashMap<i64, Vec<String>> = HashMap::new();
    for row in query.fetch_all(pool).await? {
        map.entry(row.get::<i64, _>(0))
            .or_default()
            .push(row.get(1));
    }
    Ok(map)
}

#[derive(Deserialize)]
struct ManaPoolFeed {
    #[serde(default)]
    data: Vec<ManaPoolProduct>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ManaPoolProduct {
    tcgplayer_product_id: Option<i64>,
    language_id: Option<String>,
    low_price: Option<i64>,
}

pub async fn import_manapool_sealed(
    pool: &SqlitePool,
    client: &reqwest::Client,
) -> anyhow::Result<usize> {
    let body = crate::http::get_bytes(client, MANAPOOL_SEALED_URL).await?;
    let feed: ManaPoolFeed = serde_json::from_slice(&body).context("parse ManaPool sealed feed")?;
    let by_tcg = products_by_id(pool, VendorId::TcgPlayer).await?;
    let mut rows: HashMap<String, u32> = HashMap::new();
    for product in feed.data {
        if product.language_id.as_deref().is_some_and(|l| l != "EN") {
            continue;
        }
        let (Some(id), Some(price)) = (product.tcgplayer_product_id, product.low_price) else {
            continue;
        };
        if price <= 0 {
            continue;
        }
        for uuid in by_tcg.get(&id).into_iter().flatten() {
            let entry = rows.entry(uuid.clone()).or_insert(price as u32);
            *entry = (*entry).min(price as u32);
        }
    }
    let rows: Vec<(String, u32)> = rows.into_iter().collect();
    replace_sealed_prices(pool, Vendor::ManaPool, &rows).await?;
    Ok(rows.len())
}

#[derive(Deserialize)]
struct CardKingdomFeed {
    #[serde(default)]
    data: Vec<CardKingdomProduct>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CardKingdomProduct {
    id: Option<i64>,
    price_retail: Option<String>,
    qty_retail: Option<i64>,
}

pub async fn import_card_kingdom_sealed(
    pool: &SqlitePool,
    client: &reqwest::Client,
) -> anyhow::Result<usize> {
    let body = crate::http::get_bytes(client, CARD_KINGDOM_SEALED_URL).await?;
    let feed: CardKingdomFeed =
        serde_json::from_slice(&body).context("parse Card Kingdom sealed feed")?;
    let by_ck = products_by_id(pool, VendorId::CardKingdom).await?;
    let mut rows = Vec::new();
    for product in feed.data {
        let (Some(id), Some(price)) = (product.id, product.price_retail) else {
            continue;
        };
        // Out-of-stock listings keep a stale price; skip them.
        if product.qty_retail.unwrap_or(0) <= 0 {
            continue;
        }
        let Some(cents) = price.parse::<f64>().ok().and_then(cents).filter(|c| *c > 0) else {
            continue;
        };
        for uuid in by_ck.get(&id).into_iter().flatten() {
            rows.push((uuid.clone(), cents));
        }
    }
    replace_sealed_prices(pool, Vendor::CardKingdom, &rows).await?;
    Ok(rows.len())
}

#[derive(Deserialize)]
struct TcgCsvPrices {
    #[serde(default)]
    results: Vec<TcgCsvPrice>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct TcgCsvPrice {
    #[serde(rename = "productId")]
    product_id: Option<i64>,
    #[serde(rename = "marketPrice")]
    market_price: Option<f64>,
    #[serde(rename = "lowPrice")]
    low_price: Option<f64>,
    #[serde(rename = "subTypeName")]
    sub_type_name: Option<String>,
}

/// TCGplayer market prices for sealed products, from the price list of each
/// set's TCGplayer group.
pub async fn import_tcgcsv_sealed(
    pool: &SqlitePool,
    client: &reqwest::Client,
    progress: Progress,
) -> anyhow::Result<usize> {
    let by_tcg = products_by_id(pool, VendorId::TcgPlayer).await?;
    let groups: Vec<i64> = sqlx::query(
        "SELECT DISTINCT s.tcgplayer_group_id FROM sets s
         JOIN sealed_products p ON p.set_code = s.code
         WHERE s.tcgplayer_group_id IS NOT NULL AND p.tcgplayer_product_id IS NOT NULL",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| row.get::<i64, _>(0))
    .collect();
    let mut rows = Vec::new();
    for (index, group) in groups.iter().enumerate() {
        if index % 25 == 0 {
            progress(format!(
                "Downloading TCGplayer sealed prices: group {}/{}",
                index + 1,
                groups.len()
            ));
        }
        tokio::time::sleep(TCGCSV_DELAY).await;
        let url = format!("{TCGCSV_BASE_URL}/{group}/prices");
        let body = match crate::http::get_bytes(client, &url).await {
            Ok(body) => body,
            Err(error) => {
                tracing::warn!(%error, group, "tcgcsv group skipped");
                continue;
            }
        };
        let Ok(prices) = serde_json::from_slice::<TcgCsvPrices>(&body) else {
            tracing::warn!(group, "tcgcsv group skipped: unexpected payload");
            continue;
        };
        for price in prices.results {
            let Some(id) = price.product_id else { continue };
            let Some(uuids) = by_tcg.get(&id) else {
                continue;
            };
            if price
                .sub_type_name
                .as_deref()
                .is_some_and(|s| s != "Normal")
            {
                continue;
            }
            let Some(cents) = price
                .market_price
                .or(price.low_price)
                .and_then(cents)
                .filter(|c| *c > 0)
            else {
                continue;
            };
            for uuid in uuids {
                rows.push((uuid.clone(), cents));
            }
        }
    }
    replace_sealed_prices(pool, Vendor::TcgPlayer, &rows).await?;
    Ok(rows.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_price_rows_take_the_latest_usd_retail_price_per_finish() {
        let json = r#"{"data":{"abc":{"paper":{
            "tcgplayer":{"retail":{"normal":{"2026-11-06":1.5,"2026-11-07":2.25},"foil":{"2026-11-07":3}},"currency":"USD"},
            "cardmarket":{"retail":{"normal":{"2026-11-07":9}},"currency":"EUR"},
            "cardkingdom":{"retail":{"etched":{"2026-11-07":0.49}},"buylist":{"normal":{"2026-11-07":99}},"currency":"USD"}
        }}}}"#;
        let file: PriceFile = serde_json::from_str(json).unwrap();
        let mut rows = card_price_rows(&file);
        rows.sort_by_key(|r| (r.vendor.as_str(), r.finish));
        assert_eq!(
            rows,
            vec![
                CardPriceRow {
                    uuid: "abc",
                    vendor: Vendor::CardKingdom,
                    finish: "etched",
                    cents: 49
                },
                CardPriceRow {
                    uuid: "abc",
                    vendor: Vendor::TcgPlayer,
                    finish: "foil",
                    cents: 300
                },
                CardPriceRow {
                    uuid: "abc",
                    vendor: Vendor::TcgPlayer,
                    finish: "nonfoil",
                    cents: 225
                },
            ]
        );
    }
}
