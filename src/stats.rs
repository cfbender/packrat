//! Expected value: closed-form moments of a pack's value from sheet weights
//! and prices, Monte Carlo summaries, and product EV built up from the
//! packs a sealed product contains.

use std::collections::HashSet;

use rand::Rng;
use serde::Serialize;

use crate::catalog::{Booster, Catalog, Pricing};
use crate::sim::open_pack;

/// Mean and variance of a value in dollars.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Moments {
    pub mean: f64,
    pub variance: f64,
}

impl Moments {
    pub fn sd(&self) -> f64 {
        self.variance.max(0.0).sqrt()
    }

    fn scaled(self, count: f64) -> Self {
        Moments {
            mean: self.mean * count,
            variance: self.variance * count,
        }
    }

    fn plus(self, other: Self) -> Self {
        Moments {
            mean: self.mean + other.mean,
            variance: self.variance + other.variance,
        }
    }
}

/// Theoretical pack moments. Picks within a slot are treated as independent
/// draws (with replacement), which is exact for the mean and a close
/// approximation of the variance for sheets much larger than the pick
/// count. Cards without a price count as $0; `missing` reports how many
/// distinct cards that affected.
pub struct Theoretical {
    pub moments: Moments,
    pub missing: usize,
    pub priced: usize,
}

pub fn theoretical(booster: &Booster, catalog: &Catalog, pricing: Pricing) -> Theoretical {
    let mut missing = HashSet::new();
    let mut priced = HashSet::new();
    let sheet_moments: Vec<Moments> = booster
        .sheets
        .iter()
        .map(|sheet| {
            if sheet.total_weight == 0 {
                return Moments::default();
            }
            let mut mean = 0.0;
            let mut second = 0.0;
            for card in &sheet.cards {
                let price = match catalog.priced(pricing, &card.uuid, sheet.foil) {
                    Some(cents) => {
                        priced.insert(card.uuid.clone());
                        cents as f64 / 100.0
                    }
                    None => {
                        missing.insert(card.uuid.clone());
                        0.0
                    }
                };
                let p = card.weight as f64 / sheet.total_weight as f64;
                mean += p * price;
                second += p * price * price;
            }
            Moments {
                mean,
                variance: (second - mean * mean).max(0.0),
            }
        })
        .collect();

    let total = booster.total_weight as f64;
    let mut mean = 0.0;
    let mut second = 0.0;
    for variant in &booster.variants {
        let q = variant.weight as f64 / total;
        let m = variant
            .slots
            .iter()
            .map(|slot| sheet_moments[slot.sheet].scaled(slot.picks as f64))
            .fold(Moments::default(), Moments::plus);
        mean += q * m.mean;
        second += q * (m.variance + m.mean * m.mean);
    }
    Theoretical {
        moments: Moments {
            mean,
            variance: (second - mean * mean).max(0.0),
        },
        missing: missing.len(),
        priced: priced.len(),
    }
}

/// Dollar value of one opened pack.
pub fn pack_value(cards: &[crate::sim::PulledCard], catalog: &Catalog, pricing: Pricing) -> f64 {
    cards
        .iter()
        .filter_map(|c| catalog.priced(pricing, &c.uuid, c.foil))
        .map(|cents| cents as f64 / 100.0)
        .sum()
}

#[derive(Debug, Clone, Serialize)]
pub struct Simulated {
    pub samples: usize,
    pub mean: f64,
    pub variance: f64,
    pub sd: f64,
    pub median: f64,
    pub min: f64,
    pub max: f64,
    /// Share of packs worth at least `price`, when a price is known.
    pub break_even: Option<f64>,
    pub histogram: Vec<HistogramBin>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistogramBin {
    pub from: f64,
    pub to: f64,
    pub count: usize,
}

pub fn simulate(
    booster: &Booster,
    catalog: &Catalog,
    pricing: Pricing,
    samples: usize,
    price: Option<f64>,
    rng: &mut impl Rng,
) -> Simulated {
    let mut values: Vec<f64> = (0..samples)
        .map(|_| pack_value(&open_pack(booster, rng), catalog, pricing))
        .collect();
    summarize(&mut values, price)
}

pub fn summarize(values: &mut [f64], price: Option<f64>) -> Simulated {
    values.sort_by(|a, b| a.total_cmp(b));
    let n = values.len().max(1) as f64;
    let mean = values.iter().sum::<f64>() / n;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
    let median = if values.is_empty() {
        0.0
    } else if values.len().is_multiple_of(2) {
        (values[values.len() / 2 - 1] + values[values.len() / 2]) / 2.0
    } else {
        values[values.len() / 2]
    };
    let min = values.first().copied().unwrap_or(0.0);
    let max = values.last().copied().unwrap_or(0.0);
    let break_even = price.map(|price| values.iter().filter(|v| **v >= price).count() as f64 / n);

    // Bins span the bulk of the distribution; the long tail lands in the last bin.
    let bins = 24usize;
    let cap_index = ((values.len() as f64 * 0.98) as usize).min(values.len().saturating_sub(1));
    let cap = values.get(cap_index).copied().unwrap_or(0.0).max(min + 1.0);
    let width = (cap - min) / bins as f64;
    let mut histogram: Vec<HistogramBin> = (0..bins)
        .map(|i| HistogramBin {
            from: min + width * i as f64,
            to: min + width * (i + 1) as f64,
            count: 0,
        })
        .collect();
    for &v in values.iter() {
        let index = (((v - min) / width) as usize).min(bins - 1);
        histogram[index].count += 1;
    }
    if let Some(last) = histogram.last_mut() {
        last.to = max.max(last.to);
    }

    Simulated {
        samples: values.len(),
        mean,
        variance,
        sd: variance.sqrt(),
        median,
        min,
        max,
        break_even,
        histogram,
    }
}

/// Theoretical moments of a sealed product's contents: packs through their
/// booster configuration, nested sealed products recursively, and single
/// cards at their price. `None` when any content cannot be valued.
pub fn product_moments(
    catalog: &Catalog,
    uuid: &str,
    pricing: Pricing,
    depth: usize,
) -> Option<Moments> {
    if depth > 6 {
        return None;
    }
    let product = catalog.products.get(uuid)?;
    let contents = &product.contents;
    if contents.is_empty() || contents.variable || contents.decks > 0 {
        return None;
    }
    let mut total = Moments::default();
    for pack in &contents.packs {
        let booster = catalog.booster(&pack.set, &pack.code)?;
        total = total.plus(theoretical(&booster, catalog, pricing).moments);
    }
    for sealed in &contents.sealed {
        let inner = product_moments(catalog, &sealed.uuid, pricing, depth + 1)?;
        total = total.plus(inner.scaled(sealed.count as f64));
    }
    for card in &contents.cards {
        let price = catalog.priced(pricing, &card.uuid, card.foil).unwrap_or(0);
        total = total.plus(Moments {
            mean: price as f64 / 100.0,
            variance: 0.0,
        });
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{FinishPrices, Sheet, SheetCard, Slot, Variant, Vendor};
    use std::sync::Arc;

    fn card(uuid: &str, weight: u64) -> SheetCard {
        SheetCard {
            uuid: Arc::from(uuid),
            weight,
            colors: 0,
        }
    }

    fn catalog_with_prices(prices: &[(&str, u32, u32)]) -> Catalog {
        let mut catalog = Catalog::default();
        let map = catalog.card_prices.entry(Vendor::TcgPlayer).or_default();
        for (uuid, nonfoil, foil) in prices {
            map.insert(
                Arc::from(*uuid),
                FinishPrices {
                    nonfoil: Some(*nonfoil),
                    foil: Some(*foil),
                    etched: None,
                },
            );
        }
        catalog
    }

    #[test]
    fn theoretical_mean_mixes_variants_and_sheet_weights() {
        // Rare sheet: $10 with weight 3, $2 with weight 1 -> mean $8, E[x^2] = 76, var = 12.
        // Foil sheet: one $4 (foil price) card.
        // Variant A (weight 3): 1 rare. Variant B (weight 1): 1 rare + 1 foil.
        let booster = Booster {
            set_code: "TST".into(),
            code: "play".into(),
            sheets: vec![
                Sheet {
                    name: "rare".into(),
                    foil: false,
                    balance_colors: false,
                    total_weight: 4,
                    cards: vec![card("a", 3), card("b", 1)],
                },
                Sheet {
                    name: "foil".into(),
                    foil: true,
                    balance_colors: false,
                    total_weight: 1,
                    cards: vec![card("c", 1)],
                },
            ],
            variants: vec![
                Variant {
                    weight: 3,
                    slots: vec![Slot { sheet: 0, picks: 1 }],
                },
                Variant {
                    weight: 1,
                    slots: vec![Slot { sheet: 0, picks: 1 }, Slot { sheet: 1, picks: 1 }],
                },
            ],
            total_weight: 4,
        };
        let catalog = catalog_with_prices(&[("a", 1000, 0), ("b", 200, 0), ("c", 100, 400)]);
        let result = theoretical(&booster, &catalog, Pricing::new(Vendor::TcgPlayer));
        // mean = 0.75 * 8 + 0.25 * 12 = 9
        assert!(
            (result.moments.mean - 9.0).abs() < 1e-9,
            "{}",
            result.moments.mean
        );
        // E[x^2] = 0.75 * 76 + 0.25 * (12 + 144) = 57 + 39 = 96 ; var = 96 - 81 = 15
        assert!(
            (result.moments.variance - 15.0).abs() < 1e-9,
            "{}",
            result.moments.variance
        );
        assert_eq!(result.missing, 0);
        assert_eq!(result.priced, 3);
    }

    #[test]
    fn summarize_reports_median_and_break_even() {
        let mut values = vec![5.0, 1.0, 3.0, 9.0];
        let s = summarize(&mut values, Some(4.0));
        assert_eq!(s.median, 4.0);
        assert_eq!(s.break_even, Some(0.5));
        assert_eq!(s.min, 1.0);
        assert_eq!(s.max, 9.0);
        assert_eq!(s.histogram.iter().map(|b| b.count).sum::<usize>(), 4);
    }
}
