//! Catalog import from MTGJSON's `AllPrintings.sqlite.xz`: the one file that
//! carries sets, cards, booster configurations (sheets, weights, pack
//! variants) and sealed products. The archive is downloaded and
//! decompressed into the data directory, attached to our database, copied
//! table by table, and deleted.

use std::path::{Path, PathBuf};

use anyhow::Context;
use sqlx::{Connection, Row, SqlitePool};

use crate::db;

pub type Progress = std::sync::Arc<dyn Fn(String) + Send + Sync>;

/// The MTGJSON build currently published, from `Meta.json`.
pub async fn published_version(client: &reqwest::Client, base_url: &str) -> anyhow::Result<String> {
    let body = crate::http::get_bytes(client, &format!("{base_url}/Meta.json")).await?;
    let value: serde_json::Value = serde_json::from_slice(&body)?;
    value
        .pointer("/data/version")
        .or_else(|| value.pointer("/meta/version"))
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .context("Meta.json has no version")
}

pub async fn is_catalog_empty(pool: &SqlitePool) -> anyhow::Result<bool> {
    let count: i64 = sqlx::query("SELECT count(*) FROM booster_sheets")
        .fetch_one(pool)
        .await?
        .get(0);
    Ok(count == 0)
}

/// Imports the published catalog unless the database already holds that
/// version. Returns true when anything changed.
pub async fn import(
    pool: &SqlitePool,
    client: &reqwest::Client,
    base_url: &str,
    data_dir: &Path,
    force: bool,
    progress: Progress,
) -> anyhow::Result<bool> {
    let version = published_version(client, base_url).await?;
    let current = db::meta_get(pool, "mtgjson_version").await?;
    if !force && current.as_deref() == Some(version.as_str()) && !is_catalog_empty(pool).await? {
        progress(format!("MTGJSON {version} already imported"));
        return Ok(false);
    }

    let scratch = data_dir.join("tmp");
    tokio::fs::create_dir_all(&scratch).await?;
    let archive = scratch.join("AllPrintings.sqlite.xz");
    let sqlite = scratch.join("AllPrintings.sqlite");

    let url = format!("{base_url}/AllPrintings.sqlite.xz");
    progress(format!("Downloading MTGJSON {version}"));
    {
        let progress = progress.clone();
        let version = version.clone();
        crate::http::download(client, &url, &archive, move |received, total| {
            let mb = received as f64 / 1_048_576.0;
            match total {
                Some(total) => progress(format!(
                    "Downloading MTGJSON {version}: {mb:.0} / {:.0} MB",
                    total as f64 / 1_048_576.0
                )),
                None => progress(format!("Downloading MTGJSON {version}: {mb:.0} MB")),
            }
        })
        .await?;
    }

    progress("Decompressing AllPrintings.sqlite".to_owned());
    decompress(archive.clone(), sqlite.clone()).await?;
    let _ = tokio::fs::remove_file(&archive).await;

    progress("Importing sets, cards, boosters, and sealed products".to_owned());
    let result = copy_tables(pool, &sqlite).await;
    let _ = tokio::fs::remove_file(&sqlite).await;
    result?;

    db::meta_set(pool, "mtgjson_version", &version).await?;
    db::meta_set(pool, "catalog_imported_at", &db::now()).await?;
    sqlx::query("PRAGMA optimize").execute(pool).await?;
    progress(format!("Imported MTGJSON {version}"));
    Ok(true)
}

async fn decompress(archive: PathBuf, target: PathBuf) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let input =
            std::fs::File::open(&archive).with_context(|| format!("open {}", archive.display()))?;
        let mut decoder = liblzma::read::XzDecoder::new(std::io::BufReader::new(input));
        let mut output = std::io::BufWriter::new(
            std::fs::File::create(&target)
                .with_context(|| format!("create {}", target.display()))?,
        );
        std::io::copy(&mut decoder, &mut output).context("decompress AllPrintings.sqlite.xz")?;
        Ok(())
    })
    .await??;
    Ok(())
}

const COPY_STATEMENTS: &[&str] = &[
    "DELETE FROM sets",
    "INSERT OR IGNORE INTO sets (code, name, release_date, set_type, parent_code, keyrune_code, tcgplayer_group_id)
     SELECT code, name, releaseDate, type, parentCode, keyruneCode, CAST(tcgplayerGroupId AS INTEGER)
     FROM mtgjson.sets",
    "DELETE FROM cards",
    "INSERT OR IGNORE INTO cards (uuid, scryfall_id, name, set_code, number, rarity, colors, types, layout,
       finishes, promo_types, frame_effects, border_color, tcgplayer_product_id, card_kingdom_id, card_kingdom_foil_id)
     SELECT c.uuid, i.scryfallId, c.name, c.setCode, c.number, c.rarity, c.colors, c.types, c.layout,
       c.finishes, c.promoTypes, c.frameEffects, c.borderColor,
       CAST(i.tcgplayerProductId AS INTEGER), CAST(i.cardKingdomId AS INTEGER), CAST(i.cardKingdomFoilId AS INTEGER)
     FROM mtgjson.cards c LEFT JOIN mtgjson.cardIdentifiers i ON i.uuid = c.uuid
     WHERE c.side IS NULL OR c.side = 'a'",
    "DELETE FROM booster_sheets",
    "INSERT OR IGNORE INTO booster_sheets (set_code, booster_code, sheet, is_foil, balance_colors, total_weight)
     SELECT setCode, boosterName, sheetName, COALESCE(sheetIsFoil, 0), COALESCE(sheetHasBalanceColors, 0),
       COALESCE(sheetTotalWeight, 0)
     FROM mtgjson.setBoosterSheets",
    "DELETE FROM booster_sheet_cards",
    "INSERT INTO booster_sheet_cards (set_code, booster_code, sheet, card_uuid, weight)
     SELECT setCode, boosterName, sheetName, cardUuid, COALESCE(cardWeight, 1)
     FROM mtgjson.setBoosterSheetCards",
    "DELETE FROM booster_variants",
    "INSERT OR IGNORE INTO booster_variants (set_code, booster_code, variant, weight)
     SELECT setCode, boosterName, boosterIndex, COALESCE(boosterWeight, 1)
     FROM mtgjson.setBoosterContentWeights",
    "DELETE FROM booster_variant_slots",
    "INSERT OR IGNORE INTO booster_variant_slots (set_code, booster_code, variant, sheet, picks)
     SELECT setCode, boosterName, boosterIndex, sheetName, COALESCE(sheetPicks, 1)
     FROM mtgjson.setBoosterContents",
    "DELETE FROM sealed_products",
    "INSERT OR IGNORE INTO sealed_products (uuid, set_code, name, category, subtype, card_count, contents,
       tcgplayer_product_id, card_kingdom_id, release_date)
     SELECT uuid, setCode, name, category, subtype, cardCount, contents,
       CAST(json_extract(identifiers, '$.tcgplayerProductId') AS INTEGER),
       CAST(json_extract(identifiers, '$.cardKingdomId') AS INTEGER),
       releaseDate
     FROM mtgjson.sealedProducts",
];

async fn copy_tables(pool: &SqlitePool, sqlite: &Path) -> anyhow::Result<()> {
    let mut conn = pool.acquire().await?;
    sqlx::query("ATTACH DATABASE ? AS mtgjson")
        .bind(sqlite.to_string_lossy().to_string())
        .execute(&mut *conn)
        .await
        .context("attach AllPrintings.sqlite")?;
    let result = async {
        let mut tx = conn.begin().await?;
        for statement in COPY_STATEMENTS {
            sqlx::query(sqlx::AssertSqlSafe(*statement))
                .execute(&mut *tx)
                .await
                .with_context(|| {
                    format!("import step: {}", statement.lines().next().unwrap_or(""))
                })?;
        }
        tx.commit().await?;
        anyhow::Ok(())
    }
    .await;
    let _ = sqlx::query("DETACH DATABASE mtgjson")
        .execute(&mut *conn)
        .await;
    result
}
