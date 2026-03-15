mod print;

use clap::Parser;
use colored::Colorize;
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::File;
use std::io::{self, BufReader, Read};

#[derive(Parser)]
#[command(
    name = "jpeek",
    about = "Peek at JSON structure — types, examples, value ranges at a glance"
)]
pub(crate) struct Args {
    /// JSON file to analyze (reads stdin if omitted)
    file: Option<String>,

    /// Max length for displayed string values
    #[arg(short = 'l', long, default_value_t = 25)]
    pub(crate) max_len: usize,
}

/// Discriminant for the JSON type of a field
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum TypeKey {
    String,
    Number,
    Bool,
    Null,
    Object,
    Array,
    Undefined,
}

/// Tracks statistics for a single type occurrence of a field
#[derive(Clone, Debug)]
pub(crate) enum TypeStats {
    String {
        example: String,
        min_val: String,
        max_val: String,
    },
    Number {
        example: f64,
        min: f64,
        max: f64,
        is_float: bool,
    },
    Bool {
        example: bool,
        has_true: bool,
        has_false: bool,
    },
    Null {
        example_count: usize,
        min_count: usize,
        max_count: usize,
    },
    Undefined {
        example_count: usize,
        min_count: usize,
        max_count: usize,
    },
    Object {
        items: BTreeMap<String, CollectionStats>,
    },
    Array {
        example_len: usize,
        min_len: usize,
        max_len: usize,
        items: Box<CollectionStats>,
    },
}

impl TypeStats {
    fn merge(&mut self, other: Self) {
        match (self, other) {
            (
                Self::String {
                    min_val, max_val, ..
                },
                Self::String {
                    min_val: other_min,
                    max_val: other_max,
                    ..
                },
            ) => {
                if other_min < *min_val {
                    *min_val = other_min;
                }
                if other_max > *max_val {
                    *max_val = other_max;
                }
            }
            (
                Self::Number {
                    min, max, is_float, ..
                },
                Self::Number {
                    min: other_min,
                    max: other_max,
                    is_float: other_float,
                    ..
                },
            ) => {
                *min = min.min(other_min);
                *max = max.max(other_max);
                *is_float |= other_float;
            }
            (
                Self::Bool {
                    has_true,
                    has_false,
                    ..
                },
                Self::Bool {
                    has_true: ot,
                    has_false: of,
                    ..
                },
            ) => {
                *has_true |= ot;
                *has_false |= of;
            }
            (
                Self::Null {
                    example_count,
                    min_count,
                    max_count,
                }
                | Self::Undefined {
                    example_count,
                    min_count,
                    max_count,
                },
                Self::Null {
                    example_count: oc,
                    min_count: omin,
                    max_count: omax,
                }
                | Self::Undefined {
                    example_count: oc,
                    min_count: omin,
                    max_count: omax,
                },
            ) => {
                *example_count += oc;
                *min_count = (*min_count).min(omin);
                *max_count = (*max_count).max(omax);
            }
            (Self::Object { items }, Self::Object { items: other_items }) => {
                let other_keys: BTreeSet<_> = other_items.keys().cloned().collect();
                for (k, v) in other_items {
                    items
                        .entry(k)
                        .or_insert_with(|| {
                            let mut cs = CollectionStats::default();
                            cs.merge_value(TypeStats::Undefined {
                                example_count: 1,
                                min_count: 1,
                                max_count: 1,
                            });
                            cs
                        })
                        .merge(v);
                }
                for (k, v) in items.iter_mut() {
                    if !other_keys.contains(k) {
                        v.merge_value(TypeStats::Undefined {
                            example_count: 1,
                            min_count: 1,
                            max_count: 1,
                        });
                    }
                }
            }
            (
                Self::Array {
                    min_len,
                    max_len,
                    items,
                    ..
                },
                Self::Array {
                    min_len: other_min,
                    max_len: other_max,
                    items: other_items,
                    ..
                },
            ) => {
                *min_len = (*min_len).min(other_min);
                *max_len = (*max_len).max(other_max);
                items.merge(*other_items);
            }
            _ => {}
        }
    }

    fn type_key(&self) -> TypeKey {
        match self {
            Self::String { .. } => TypeKey::String,
            Self::Number { .. } => TypeKey::Number,
            Self::Bool { .. } => TypeKey::Bool,
            Self::Null { .. } => TypeKey::Null,
            Self::Undefined { .. } => TypeKey::Undefined,
            Self::Object { .. } => TypeKey::Object,
            Self::Array { .. } => TypeKey::Array,
        }
    }
}

/// Tracks all type variants seen for a field
#[derive(Clone, Debug, Default)]
pub(crate) struct CollectionStats {
    pub(crate) types: BTreeMap<TypeKey, TypeStats>,
}

impl CollectionStats {
    fn merge_value(&mut self, stats: TypeStats) {
        let key = stats.type_key();

        if let Some(existing) = self.types.get_mut(&key) {
            existing.merge(stats);
        } else {
            self.types.insert(key, stats);
        }
    }

    fn merge(&mut self, other: Self) {
        let self_keys: BTreeSet<_> = self.types.keys().cloned().collect();
        let other_keys: BTreeSet<_> = other.types.keys().cloned().collect();

        // Insert keys only in other
        for (key, stats) in other.types {
            if let Some(existing) = self.types.get_mut(&key) {
                existing.merge(stats);
            } else {
                self.types.insert(key, stats);
            }
        }

        // Keys in only one side: min_count becomes 0
        for key in self_keys.symmetric_difference(&other_keys) {
            if let Some(
                TypeStats::Null { min_count, .. } | TypeStats::Undefined { min_count, .. },
            ) = self.types.get_mut(key)
            {
                *min_count = 0;
            }
        }
    }
}

// --- Streaming deserializer ---

struct StatsSeed;

impl<'de> DeserializeSeed<'de> for StatsSeed {
    type Value = TypeStats;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        deserializer.deserialize_any(StatsVisitor)
    }
}

struct StatsVisitor;

impl<'de> Visitor<'de> for StatsVisitor {
    type Value = TypeStats;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<TypeStats, E> {
        Ok(TypeStats::String {
            example: v.to_owned(),
            min_val: v.to_owned(),
            max_val: v.to_owned(),
        })
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<TypeStats, E> {
        Ok(TypeStats::Bool {
            example: v,
            has_true: v,
            has_false: !v,
        })
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<TypeStats, E> {
        let f = v as f64;
        Ok(TypeStats::Number {
            example: f,
            min: f,
            max: f,
            is_float: false,
        })
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<TypeStats, E> {
        let f = v as f64;
        Ok(TypeStats::Number {
            example: f,
            min: f,
            max: f,
            is_float: false,
        })
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<TypeStats, E> {
        Ok(TypeStats::Number {
            example: v,
            min: v,
            max: v,
            is_float: v.fract() != 0.0,
        })
    }

    fn visit_unit<E: de::Error>(self) -> Result<TypeStats, E> {
        Ok(TypeStats::Null {
            example_count: 1,
            min_count: 1,
            max_count: 1,
        })
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<TypeStats, A::Error> {
        let mut items = CollectionStats::default();
        let mut len: usize = 0;
        while let Some(element) = seq.next_element_seed(StatsSeed)? {
            items.merge_value(element);
            len += 1;
        }
        Ok(TypeStats::Array {
            example_len: len,
            min_len: len,
            max_len: len,
            items: Box::new(items),
        })
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<TypeStats, A::Error> {
        let mut items = BTreeMap::new();
        while let Some(key) = map.next_key::<String>()? {
            let value = map.next_value_seed(StatsSeed)?;
            let mut cs = CollectionStats::default();
            cs.merge_value(value);
            items.insert(key, cs);
        }
        Ok(TypeStats::Object { items })
    }
}

// --- Main ---

fn main() {
    let args = Args::parse();

    let reader: Box<dyn Read> = match &args.file {
        Some(path) => {
            let file = File::open(path).unwrap_or_else(|e| {
                eprintln!("{} reading {}: {}", "error:".red().bold(), path, e);
                std::process::exit(1);
            });
            Box::new(BufReader::new(file))
        }
        None => Box::new(BufReader::new(io::stdin())),
    };

    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    let stats = StatsSeed
        .deserialize(&mut deserializer)
        .unwrap_or_else(|e| {
            eprintln!("{} invalid JSON: {}", "error:".red().bold(), e);
            std::process::exit(1);
        });
    print::print_root(&stats, &args);
}
