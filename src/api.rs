//! HTTP API consumed by the web app.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Row, SqlitePool};

use crate::catalog::{Booster, Catalog, Pricing, Product, Vendor};
use crate::config::Config;
use crate::sim::{PulledCard, open_pack};
use crate::stats;

#[derive(Debug, Clone, Default, Serialize)]
pub struct JobStatus {
    pub busy: bool,
    pub message: String,
    pub error: Option<String>,
}

pub struct AppState {
    pub pool: SqlitePool,
    pub catalog: RwLock<Arc<Catalog>>,
    pub status: Mutex<JobStatus>,
    pub config: Config,
    pub client: reqwest::Client,
    pub refresh: tokio::sync::mpsc::Sender<crate::jobs::Request>,
}

pub type SharedState = Arc<AppState>;

impl AppState {
    pub fn catalog(&self) -> Arc<Catalog> {
        self.catalog.read().expect("catalog lock").clone()
    }

    pub fn set_catalog(&self, catalog: Catalog) {
        *self.catalog.write().expect("catalog lock") = Arc::new(catalog);
    }

    pub fn set_status(&self, update: impl FnOnce(&mut JobStatus)) {
        let mut status = self.status.lock().expect("status lock");
        update(&mut status);
    }
}

pub struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        tracing::error!(%error, "request failed");
        ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Something went wrong.".to_owned(),
        )
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        ApiError::from(anyhow::Error::from(error))
    }
}

fn bad_request(message: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, message.into())
}

fn not_found(message: impl Into<String>) -> ApiError {
    ApiError(StatusCode::NOT_FOUND, message.into())
}

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/status", get(status))
        .route("/products", get(products))
        .route("/open", post(open))
        .route("/stats", get(pack_stats))
        .route("/ev", get(ev_rankings))
        .route("/refresh", post(refresh))
}

#[derive(Serialize)]
struct Status {
    ready: bool,
    busy: bool,
    message: String,
    error: Option<String>,
    mtgjson_version: Option<String>,
    prices_updated_at: Option<String>,
    booster_count: usize,
    product_count: usize,
    vendors: Vec<Vendor>,
}

async fn status(State(state): State<SharedState>) -> Json<Status> {
    let catalog = state.catalog();
    let job = state.status.lock().expect("status lock").clone();
    Json(Status {
        ready: !catalog.boosters.is_empty(),
        busy: job.busy,
        message: job.message,
        error: job.error,
        mtgjson_version: catalog.mtgjson_version.clone(),
        prices_updated_at: catalog.prices_updated_at.clone(),
        booster_count: catalog.boosters.len(),
        product_count: catalog.products.len(),
        vendors: Vendor::ALL.to_vec(),
    })
}

#[derive(Serialize)]
struct ProductSummary {
    uuid: String,
    name: String,
    count: u32,
    prices: HashMap<Vendor, u32>,
}

fn product_summary(catalog: &Catalog, product: &Product, count: u32) -> ProductSummary {
    ProductSummary {
        uuid: product.uuid.clone(),
        name: product.name.clone(),
        count,
        prices: catalog.sealed_prices_all(&product.uuid),
    }
}

#[derive(Serialize)]
struct BoosterListing {
    set_code: String,
    set_name: String,
    release_date: Option<String>,
    set_type: Option<String>,
    keyrune_code: Option<String>,
    booster_code: String,
    name: String,
    card_count: f64,
    pack: Option<ProductSummary>,
    #[serde(rename = "box")]
    box_product: Option<ProductSummary>,
}

fn booster_display_name(set_name: &str, code: &str) -> String {
    let words: Vec<String> = code
        .split('-')
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect();
    format!("{set_name} {} Booster", words.join(" "))
}

async fn products(State(state): State<SharedState>) -> Json<Vec<BoosterListing>> {
    let catalog = state.catalog();
    let mut listings: Vec<BoosterListing> = catalog
        .boosters
        .values()
        .map(|booster| listing(&catalog, booster))
        .collect();
    listings.sort_by(|a, b| {
        b.release_date
            .cmp(&a.release_date)
            .then_with(|| a.set_name.cmp(&b.set_name))
            .then_with(|| a.name.cmp(&b.name))
    });
    Json(listings)
}

fn listing(catalog: &Catalog, booster: &Booster) -> BoosterListing {
    let set = catalog.sets.get(&booster.set_code);
    let set_name = set.map_or(booster.set_code.clone(), |s| s.name.clone());
    let pack = catalog.pack_product(&booster.set_code, &booster.code);
    let box_product = pack.and_then(|p| catalog.box_product(&p.uuid));
    BoosterListing {
        set_code: booster.set_code.clone(),
        set_name: set_name.clone(),
        release_date: set.and_then(|s| s.release_date.clone()),
        set_type: set.and_then(|s| s.set_type.clone()),
        keyrune_code: set.and_then(|s| s.keyrune_code.clone()),
        booster_code: booster.code.clone(),
        name: pack.map_or_else(
            || booster_display_name(&set_name, &booster.code),
            |p| p.name.clone(),
        ),
        card_count: booster.card_count(),
        pack: pack.map(|p| product_summary(catalog, p, 1)),
        box_product: box_product.map(|(b, count)| product_summary(catalog, b, count)),
    }
}

#[derive(Deserialize)]
struct OpenRequest {
    set_code: String,
    booster_code: String,
    #[serde(default = "one")]
    count: u32,
    /// Open this booster's whole box instead of `count` packs: every pack in
    /// it, including any bundled box topper.
    #[serde(default, rename = "box")]
    whole_box: bool,
    /// Cards priced below this many dollars count as $0 in pack values.
    #[serde(default)]
    floor: f64,
}

fn one() -> u32 {
    1
}

fn floor_cents(floor: Option<f64>) -> u32 {
    (floor.unwrap_or(0.0).max(0.0) * 100.0).round() as u32
}

#[derive(Serialize)]
struct OpenedCard {
    uuid: String,
    name: String,
    set_code: String,
    number: String,
    rarity: Option<String>,
    foil: bool,
    sheet: String,
    scryfall_id: Option<String>,
    image: Option<String>,
    prices: HashMap<Vendor, u32>,
}

#[derive(Serialize)]
struct OpenedPack {
    booster_code: String,
    /// The pack product's name, e.g. "Aetherdrift Box Topper Pack".
    name: String,
    cards: Vec<OpenedCard>,
    value: HashMap<Vendor, u32>,
}

#[derive(Serialize)]
struct OpenResponse {
    set_code: String,
    booster_code: String,
    packs: Vec<OpenedPack>,
}

pub fn scryfall_image(scryfall_id: &str, size: &str) -> String {
    let a = &scryfall_id[..1];
    let b = &scryfall_id[1..2];
    format!("https://cards.scryfall.io/{size}/front/{a}/{b}/{scryfall_id}.jpg")
}

struct CardDetails {
    name: String,
    set_code: String,
    number: String,
    rarity: Option<String>,
    scryfall_id: Option<String>,
}

async fn card_details(
    pool: &SqlitePool,
    uuids: &[&str],
) -> sqlx::Result<HashMap<String, CardDetails>> {
    let mut details = HashMap::new();
    for chunk in uuids.chunks(400) {
        let mut builder: QueryBuilder<sqlx::Sqlite> = QueryBuilder::new(
            "SELECT uuid, name, set_code, number, rarity, scryfall_id FROM cards WHERE uuid IN (",
        );
        let mut separated = builder.separated(", ");
        for uuid in chunk {
            separated.push_bind(*uuid);
        }
        separated.push_unseparated(")");
        for row in builder.build().fetch_all(pool).await? {
            details.insert(
                row.get::<String, _>(0),
                CardDetails {
                    name: row.get(1),
                    set_code: row.get(2),
                    number: row.get(3),
                    rarity: row.get(4),
                    scryfall_id: row.get(5),
                },
            );
        }
    }
    Ok(details)
}

async fn open(
    State(state): State<SharedState>,
    Json(request): Json<OpenRequest>,
) -> Result<Json<OpenResponse>, ApiError> {
    let catalog = state.catalog();
    let booster = catalog
        .booster(&request.set_code, &request.booster_code)
        .ok_or_else(|| not_found("Unknown booster"))?;
    // (booster, pack name, how many of it) to open, main packs first.
    let to_open: Vec<(Arc<Booster>, String, u32)> = request
        .whole_box
        .then(|| {
            let pack = catalog.pack_product(&booster.set_code, &booster.code)?;
            let (box_product, _) = catalog.box_product(&pack.uuid)?;
            Some(catalog.box_contents(&box_product.uuid))
        })
        .flatten()
        .filter(|contents| !contents.is_empty())
        .unwrap_or_else(|| {
            let name = catalog
                .pack_product(&booster.set_code, &booster.code)
                .map_or_else(|| booster.code.clone(), |p| p.name.clone());
            vec![(booster.clone(), name, request.count.clamp(1, 36))]
        });
    let pulls: Vec<(&Arc<Booster>, &str, Vec<PulledCard>)> = {
        let mut rng = rand::rng();
        let mut pulls = Vec::new();
        for (booster, name, count) in &to_open {
            for _ in 0..*count {
                pulls.push((booster, name.as_str(), open_pack(booster, &mut rng)));
            }
        }
        pulls
    };
    let mut uuids: Vec<&str> = pulls
        .iter()
        .flat_map(|(_, _, cards)| cards.iter().map(|c| c.uuid.as_ref()))
        .collect();
    uuids.sort_unstable();
    uuids.dedup();
    let details = card_details(&state.pool, &uuids).await?;

    let packs = pulls
        .iter()
        .map(|(booster, name, pack)| {
            let cards: Vec<OpenedCard> = pack
                .iter()
                .map(|pulled| {
                    let d = details.get(pulled.uuid.as_ref());
                    let scryfall_id = d.and_then(|d| d.scryfall_id.clone());
                    OpenedCard {
                        uuid: pulled.uuid.to_string(),
                        name: d.map_or_else(|| "Unknown card".to_owned(), |d| d.name.clone()),
                        set_code: d
                            .map_or_else(|| booster.set_code.clone(), |d| d.set_code.clone()),
                        number: d.map_or_else(String::new, |d| d.number.clone()),
                        rarity: d.and_then(|d| d.rarity.clone()),
                        foil: pulled.foil,
                        sheet: booster.sheets[pulled.sheet].name.clone(),
                        image: scryfall_id
                            .as_deref()
                            .filter(|id| id.len() >= 2)
                            .map(|id| scryfall_image(id, "normal")),
                        scryfall_id,
                        prices: catalog.card_prices_all(&pulled.uuid, pulled.foil),
                    }
                })
                .collect();
            let mut value: HashMap<Vendor, u32> = HashMap::new();
            let floor = floor_cents(Some(request.floor));
            for vendor in Vendor::ALL {
                let total: u32 = cards
                    .iter()
                    .filter_map(|c| c.prices.get(&vendor))
                    .filter(|&&cents| cents >= floor)
                    .sum();
                if cards.iter().any(|c| c.prices.contains_key(&vendor)) {
                    value.insert(vendor, total);
                }
            }
            OpenedPack {
                booster_code: booster.code.clone(),
                name: (*name).to_owned(),
                cards,
                value,
            }
        })
        .collect();

    Ok(Json(OpenResponse {
        set_code: booster.set_code.clone(),
        booster_code: booster.code.clone(),
        packs,
    }))
}

#[derive(Deserialize)]
struct StatsQuery {
    set_code: String,
    booster_code: String,
    vendor: Option<String>,
    samples: Option<usize>,
    floor: Option<f64>,
}

#[derive(Serialize)]
struct TheoreticalStats {
    ev: f64,
    variance: f64,
    sd: f64,
    priced_cards: usize,
    unpriced_cards: usize,
}

#[derive(Serialize)]
struct BoxExtra {
    name: String,
    count: u32,
}

#[derive(Serialize)]
struct BoxStats {
    uuid: String,
    name: String,
    count: u32,
    /// Packs bundled with the main ones, such as a box topper.
    extras: Vec<BoxExtra>,
    price: Option<f64>,
    ev: f64,
    sd: f64,
    simulated: stats::Simulated,
}

#[derive(Serialize)]
struct StatsResponse {
    vendor: Vendor,
    floor: f64,
    set_code: String,
    booster_code: String,
    card_count: f64,
    pack_price: Option<f64>,
    theoretical: TheoreticalStats,
    simulated: stats::Simulated,
    #[serde(rename = "box")]
    box_stats: Option<BoxStats>,
}

fn parse_vendor(vendor: Option<&str>) -> Result<Vendor, ApiError> {
    match vendor {
        None => Ok(Vendor::TcgPlayer),
        Some(v) => Vendor::parse(v).ok_or_else(|| bad_request(format!("Unknown vendor {v}"))),
    }
}

async fn pack_stats(
    State(state): State<SharedState>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<StatsResponse>, ApiError> {
    let catalog = state.catalog();
    let vendor = parse_vendor(query.vendor.as_deref())?;
    let pricing = Pricing::new(vendor).with_floor(floor_cents(query.floor));
    let booster = catalog
        .booster(&query.set_code, &query.booster_code)
        .ok_or_else(|| not_found("Unknown booster"))?;
    let samples = query.samples.unwrap_or(1000).clamp(100, 10_000);
    let pack = catalog.pack_product(&booster.set_code, &booster.code);
    let pack_price = pack
        .and_then(|p| catalog.sealed_price(vendor, &p.uuid))
        .map(|c| c as f64 / 100.0);
    let box_product = pack
        .and_then(|p| catalog.box_product(&p.uuid))
        .map(|(b, count)| (b.uuid.clone(), b.name.clone(), count));

    let response = tokio::task::spawn_blocking(move || {
        let theoretical = stats::theoretical(&booster, &catalog, pricing);
        let mut rng = rand::rng();
        let simulated = stats::simulate(&booster, &catalog, pricing, samples, pack_price, &mut rng);
        let box_stats = box_product.map(|(uuid, name, count)| {
            let price = catalog
                .sealed_price(vendor, &uuid)
                .map(|c| c as f64 / 100.0);
            // Main packs plus any bundled extras, e.g. a box topper.
            let contents = catalog.box_contents(&uuid);
            let extras = contents
                .iter()
                .skip(1)
                .map(|(_, name, count)| BoxExtra { name: name.clone(), count: *count })
                .collect();
            let boxes = (samples / 4).clamp(100, 1000);
            let mut values: Vec<f64> = (0..boxes)
                .map(|_| {
                    contents
                        .iter()
                        .flat_map(|(b, _, n)| (0..*n).map(move |_| b))
                        .map(|b| stats::pack_value(&open_pack(b, &mut rng), &catalog, pricing))
                        .sum()
                })
                .collect();
            let moments = contents
                .iter()
                .map(|(b, _, n)| stats::theoretical(b, &catalog, pricing).moments.scaled(*n as f64))
                .fold(stats::Moments::default(), |acc, m| acc.plus(m));
            BoxStats {
                uuid,
                name,
                count,
                extras,
                price,
                ev: moments.mean,
                sd: moments.sd(),
                simulated: stats::summarize(&mut values, price),
            }
        });
        StatsResponse {
            vendor,
            floor: pricing.floor_cents as f64 / 100.0,
            set_code: booster.set_code.clone(),
            booster_code: booster.code.clone(),
            card_count: booster.card_count(),
            pack_price,
            theoretical: TheoreticalStats {
                ev: theoretical.moments.mean,
                variance: theoretical.moments.variance,
                sd: theoretical.moments.sd(),
                priced_cards: theoretical.priced,
                unpriced_cards: theoretical.missing,
            },
            simulated,
            box_stats,
        }
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))?;
    Ok(Json(response))
}

#[derive(Deserialize)]
struct EvQuery {
    vendor: Option<String>,
    floor: Option<f64>,
}

#[derive(Serialize)]
struct EvRanking {
    uuid: String,
    name: String,
    set_code: String,
    set_name: String,
    category: Option<String>,
    subtype: Option<String>,
    release_date: Option<String>,
    /// The booster this product opens into, for jumping to the simulator.
    booster_set_code: Option<String>,
    booster_code: Option<String>,
    ev: f64,
    sd: f64,
    price: f64,
    ratio: f64,
}

async fn ev_rankings(
    State(state): State<SharedState>,
    Query(query): Query<EvQuery>,
) -> Result<Json<Vec<EvRanking>>, ApiError> {
    let catalog = state.catalog();
    let vendor = parse_vendor(query.vendor.as_deref())?;
    let pricing = Pricing::new(vendor).with_floor(floor_cents(query.floor));
    let rankings = tokio::task::spawn_blocking(move || {
        let mut rankings: Vec<EvRanking> = catalog
            .products
            .values()
            // Secret Lair drops are fixed card lists, not a gamble; keep them
            // out of a ranking of what to crack.
            .filter(|product| product.set_code != "SLD")
            .filter_map(|product| {
                let price = catalog.sealed_price(vendor, &product.uuid)? as f64 / 100.0;
                let moments = stats::product_moments(&catalog, &product.uuid, pricing, 0)?;
                if moments.mean <= 0.0 {
                    return None;
                }
                let set = catalog.sets.get(&product.set_code);
                let booster = catalog.primary_booster(&product.uuid);
                Some(EvRanking {
                    uuid: product.uuid.clone(),
                    name: product.name.clone(),
                    set_code: product.set_code.clone(),
                    set_name: set.map_or(product.set_code.clone(), |s| s.name.clone()),
                    category: product.category.clone(),
                    subtype: product.subtype.clone(),
                    release_date: product
                        .release_date
                        .clone()
                        .or_else(|| set.and_then(|s| s.release_date.clone())),
                    booster_set_code: booster.as_ref().map(|b| b.set_code.clone()),
                    booster_code: booster.as_ref().map(|b| b.code.clone()),
                    ev: moments.mean,
                    sd: moments.sd(),
                    price,
                    ratio: moments.mean / price,
                })
            })
            .collect();
        rankings.sort_by(|a, b| b.ratio.total_cmp(&a.ratio));
        rankings
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))?;
    Ok(Json(rankings))
}

#[derive(Deserialize, Default)]
struct RefreshRequest {
    #[serde(default)]
    catalog: bool,
}

async fn refresh(
    State(state): State<SharedState>,
    body: Option<Json<RefreshRequest>>,
) -> Result<Json<JobStatus>, ApiError> {
    let request = body.map(|Json(r)| r).unwrap_or_default();
    let job = if request.catalog {
        crate::jobs::Request::Catalog { force: true }
    } else {
        crate::jobs::Request::Prices
    };
    state.refresh.try_send(job).map_err(|_| {
        ApiError(
            StatusCode::CONFLICT,
            "A refresh is already running.".to_owned(),
        )
    })?;
    Ok(Json(state.status.lock().expect("status lock").clone()))
}
