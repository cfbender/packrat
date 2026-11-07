//! Opening packs: pick a pack variant by weight, then for each slot draw
//! cards from its sheet by card weight, without replacement.

use std::sync::Arc;

use rand::Rng;
use rand::distr::Distribution;
use rand::distr::weighted::WeightedIndex;

use crate::catalog::{Booster, COLOR_BITS, Sheet};

#[derive(Debug, Clone)]
pub struct PulledCard {
    pub uuid: Arc<str>,
    pub foil: bool,
    pub sheet: usize,
}

pub fn open_pack(booster: &Booster, rng: &mut impl Rng) -> Vec<PulledCard> {
    let Some(variant) = pick_variant(booster, rng) else {
        return Vec::new();
    };
    let mut cards = Vec::new();
    for slot in &variant.slots {
        let sheet = &booster.sheets[slot.sheet];
        for index in pick_from_sheet(sheet, slot.picks as usize, rng) {
            cards.push(PulledCard {
                uuid: sheet.cards[index].uuid.clone(),
                foil: sheet.foil,
                sheet: slot.sheet,
            });
        }
    }
    cards
}

fn pick_variant<'a>(
    booster: &'a Booster,
    rng: &mut impl Rng,
) -> Option<&'a crate::catalog::Variant> {
    let weights: Vec<u64> = booster.variants.iter().map(|v| v.weight).collect();
    let index = WeightedIndex::new(&weights).ok()?.sample(rng);
    booster.variants.get(index)
}

/// Indexes into `sheet.cards` for `picks` draws without replacement. A sheet
/// smaller than the pick count repeats cards (with replacement) to fill
/// the slot. Color-balanced sheets guarantee one mono-colored card of each
/// color among the first five picks, as print sheets do for commons.
pub fn pick_from_sheet(sheet: &Sheet, picks: usize, rng: &mut impl Rng) -> Vec<usize> {
    if picks == 0 || sheet.cards.is_empty() {
        return Vec::new();
    }
    let mut chosen = Vec::with_capacity(picks);
    let mut available: Vec<bool> = vec![true; sheet.cards.len()];

    if sheet.balance_colors && picks >= 5 {
        for (_, bit) in COLOR_BITS {
            let candidates: Vec<usize> = (0..sheet.cards.len())
                .filter(|&i| {
                    available[i] && sheet.cards[i].colors == bit && sheet.cards[i].weight > 0
                })
                .collect();
            if let Some(index) = weighted_pick(sheet, &candidates, rng) {
                available[index] = false;
                chosen.push(index);
            }
        }
    }

    while chosen.len() < picks {
        let candidates: Vec<usize> = (0..sheet.cards.len())
            .filter(|&i| available[i] && sheet.cards[i].weight > 0)
            .collect();
        match weighted_pick(sheet, &candidates, rng) {
            Some(index) => {
                available[index] = false;
                chosen.push(index);
            }
            None => {
                // Sheet exhausted: draw with replacement from the whole sheet.
                let all: Vec<usize> = (0..sheet.cards.len())
                    .filter(|&i| sheet.cards[i].weight > 0)
                    .collect();
                match weighted_pick(sheet, &all, rng) {
                    Some(index) => chosen.push(index),
                    None => break,
                }
            }
        }
    }
    chosen
}

fn weighted_pick(sheet: &Sheet, candidates: &[usize], rng: &mut impl Rng) -> Option<usize> {
    if candidates.is_empty() {
        return None;
    }
    let weights: Vec<u64> = candidates.iter().map(|&i| sheet.cards[i].weight).collect();
    let dist = WeightedIndex::new(&weights).ok()?;
    Some(candidates[dist.sample(rng)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{SheetCard, Slot, Variant};
    use rand::SeedableRng;
    use std::collections::HashMap;

    fn card(uuid: &str, weight: u64, colors: u8) -> SheetCard {
        SheetCard {
            uuid: Arc::from(uuid),
            weight,
            colors,
        }
    }

    fn booster() -> Booster {
        Booster {
            set_code: "TST".into(),
            code: "play".into(),
            sheets: vec![
                Sheet {
                    name: "rare".into(),
                    foil: false,
                    balance_colors: false,
                    total_weight: 8,
                    cards: vec![card("r1", 7, 0), card("m1", 1, 0)],
                },
                Sheet {
                    name: "common".into(),
                    foil: false,
                    balance_colors: true,
                    total_weight: 7,
                    cards: vec![
                        card("w", 1, 1),
                        card("u", 1, 2),
                        card("b", 1, 4),
                        card("r", 1, 8),
                        card("g", 1, 16),
                        card("c1", 1, 0),
                        card("c2", 1, 0),
                    ],
                },
                Sheet {
                    name: "foil".into(),
                    foil: true,
                    balance_colors: false,
                    total_weight: 1,
                    cards: vec![card("f", 1, 0)],
                },
            ],
            variants: vec![
                Variant {
                    weight: 9,
                    slots: vec![Slot { sheet: 0, picks: 1 }, Slot { sheet: 1, picks: 6 }],
                },
                Variant {
                    weight: 1,
                    slots: vec![
                        Slot { sheet: 0, picks: 1 },
                        Slot { sheet: 1, picks: 5 },
                        Slot { sheet: 2, picks: 1 },
                    ],
                },
            ],
            total_weight: 10,
        }
    }

    #[test]
    fn packs_follow_variant_and_sheet_weights() {
        let booster = booster();
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        let mut mythics = 0;
        let mut foils = 0;
        let mut counts: HashMap<String, usize> = HashMap::new();
        let n = 20_000;
        for _ in 0..n {
            let pack = open_pack(&booster, &mut rng);
            assert!(pack.len() == 7, "every variant yields 7 cards");
            let names: Vec<&str> = pack.iter().map(|c| c.uuid.as_ref()).collect();
            // Commons are drawn without replacement.
            let mut unique = names.clone();
            unique.sort();
            unique.dedup();
            assert_eq!(unique.len(), names.len());
            // Color balance: every mono color appears.
            for c in ["w", "u", "b", "r", "g"] {
                assert!(names.contains(&c), "pack lacks {c}: {names:?}");
            }
            mythics += usize::from(names.contains(&"m1"));
            foils += usize::from(pack.iter().any(|c| c.foil));
            for name in names {
                *counts.entry(name.to_owned()).or_default() += 1;
            }
        }
        let mythic_rate = mythics as f64 / n as f64;
        assert!(
            (mythic_rate - 0.125).abs() < 0.01,
            "mythic rate {mythic_rate}"
        );
        let foil_rate = foils as f64 / n as f64;
        assert!((foil_rate - 0.1).abs() < 0.01, "foil rate {foil_rate}");
    }

    #[test]
    fn small_sheets_fill_slots_with_replacement() {
        let mut booster = booster();
        booster.variants = vec![Variant {
            weight: 1,
            slots: vec![Slot { sheet: 2, picks: 3 }],
        }];
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        let pack = open_pack(&booster, &mut rng);
        assert_eq!(pack.len(), 3);
        assert!(pack.iter().all(|c| c.uuid.as_ref() == "f" && c.foil));
    }
}
