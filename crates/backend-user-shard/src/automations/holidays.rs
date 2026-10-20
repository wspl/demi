//! The public holidays a task's first or last working day skips.
//!
//! The table was exported once from the `holidays` Python package (MIT,
//! `LICENSE-holidays` beside it) for every country and region it covers, 2026
//! through 2077, the end of the package's own Islamic calendar tables
//! (`decisions/0010-automation-tasks-and-delivery.md`). Nothing fetches
//! holidays at run time: a day off announced after the export is missing until
//! the table is exported again. Days a government declares working days on a
//! weekend are not in it.
//!
//! The file is tab-separated text, compressed with zstd, read once on first
//! use. After a `#` line naming the package's version:
//!
//! ```text
//! C  <country>  <weekend>  <holidays>
//! R  <country>  <region>  <name>  <weekend>  <added>  <removed>
//! ```
//!
//! A weekend is its ISO weekdays, `67` for Saturday and Sunday. Holidays are
//! days counted from 2026-01-01, each written as the difference from the one
//! before, joined by commas; a region's are its country's with `added` and
//! without `removed`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use demi_web_api_protocol::automations::{HolidayCountry, HolidayPlace, HolidayRegion};
use jiff::ToSpan;
use jiff::civil::{Date, date};

const TABLE: &[u8] = include_bytes!("holidays.tsv.zst");

/// The day the table counts from.
const FIRST: Date = date(2026, 1, 1);

/// Where working days are counted: the weekdays off and the holidays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Calendar {
    /// One bit per weekday off, Monday the lowest.
    weekend: u8,
    holidays: BTreeSet<Date>,
}

impl Calendar {
    /// Monday to Friday, without holidays: a working day with no country.
    pub(crate) fn weekdays() -> Self {
        Self {
            weekend: weekend_bits("67").expect("Saturday and Sunday are weekdays"),
            holidays: BTreeSet::new(),
        }
    }

    pub(crate) fn works_on(&self, day: Date) -> bool {
        self.weekend & (1 << day.weekday().to_monday_zero_offset()) == 0 && !self.holidays.contains(&day)
    }
}

struct Country {
    weekend: u8,
    holidays: Vec<i32>,
    regions: BTreeMap<String, Region>,
}

struct Region {
    name: String,
    weekend: u8,
    added: Vec<i32>,
    removed: Vec<i32>,
}

static COUNTRIES: LazyLock<BTreeMap<String, Country>> =
    LazyLock::new(|| read(TABLE).expect("the holidays' table that ships with the backend reads"));

/// The calendar of a place, none for a country or a region the table lacks.
pub(crate) fn calendar(place: &HolidayPlace) -> Option<Calendar> {
    let country = COUNTRIES.get(&place.country)?;
    let days = |list: &[i32]| -> BTreeSet<Date> { list.iter().filter_map(|day| FIRST.checked_add(day.days()).ok()).collect() };
    let mut holidays = days(&country.holidays);
    let weekend = match &place.region {
        None => country.weekend,
        Some(code) => {
            let region = country.regions.get(code)?;
            holidays.extend(days(&region.added));
            for day in days(&region.removed) {
                holidays.remove(&day);
            }
            region.weekend
        }
    };
    Some(Calendar { weekend, holidays })
}

/// Every country the table has, with its regions, for the page's menus.
pub(crate) fn countries() -> Vec<HolidayCountry> {
    COUNTRIES
        .iter()
        .map(|(code, country)| HolidayCountry {
            code: code.clone(),
            regions: country
                .regions
                .iter()
                .map(|(code, region)| HolidayRegion {
                    code: code.clone(),
                    name: region.name.clone(),
                })
                .collect(),
        })
        .collect()
}

fn weekend_bits(text: &str) -> Option<u8> {
    text.chars().try_fold(0u8, |bits, day| {
        let iso = day.to_digit(10).filter(|day| (1..=7).contains(day))?;
        Some(bits | 1 << (iso - 1))
    })
}

fn days(text: &str) -> Option<Vec<i32>> {
    let mut day = 0;
    text.split(',')
        .filter(|part| !part.is_empty())
        .map(|part| {
            day += part.parse::<i32>().ok()?;
            Some(day)
        })
        .collect()
}

fn read(compressed: &[u8]) -> Result<BTreeMap<String, Country>, String> {
    let text = zstd::decode_all(compressed).map_err(|error| error.to_string())?;
    let text = String::from_utf8(text).map_err(|error| error.to_string())?;
    let mut countries = BTreeMap::new();
    for (number, line) in text.lines().enumerate() {
        let bad = || format!("line {}: {line:.60}", number + 1);
        let fields: Vec<&str> = line.split('\t').collect();
        match fields.as_slice() {
            [comment, ..] if comment.starts_with('#') => {}
            ["C", code, weekend, holidays] => {
                countries.insert(
                    (*code).to_owned(),
                    Country {
                        weekend: weekend_bits(weekend).ok_or_else(bad)?,
                        holidays: days(holidays).ok_or_else(bad)?,
                        regions: BTreeMap::new(),
                    },
                );
            }
            ["R", country, code, name, weekend, added, removed] => {
                let region = Region {
                    name: (*name).to_owned(),
                    weekend: weekend_bits(weekend).ok_or_else(bad)?,
                    added: days(added).ok_or_else(bad)?,
                    removed: days(removed).ok_or_else(bad)?,
                };
                countries
                    .get_mut(*country)
                    .ok_or_else(bad)?
                    .regions
                    .insert((*code).to_owned(), region);
            }
            _ => return Err(bad()),
        }
    }
    Ok(countries)
}
