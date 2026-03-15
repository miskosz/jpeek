mod print;

use clap::Parser;
use colored::Colorize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
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
    fn new(val: &Value) -> Self {
        match val {
            Value::String(s) => Self::String {
                example: s.clone(),
                min_val: s.clone(),
                max_val: s.clone(),
            },
            Value::Number(n) => {
                let f = n.as_f64().unwrap_or(0.0);
                Self::Number {
                    example: f,
                    min: f,
                    max: f,
                    is_float: n.is_f64() && f.fract() != 0.0,
                }
            }
            Value::Bool(b) => Self::Bool {
                example: *b,
                has_true: *b,
                has_false: !*b,
            },
            Value::Null => Self::Null {
                example_count: 1,
                min_count: 1,
                max_count: 1,
            },
            Value::Object(map) => {
                let mut items = BTreeMap::new();
                for (k, v) in map {
                    let mut cs = CollectionStats::default();
                    cs.merge_value(TypeStats::new(v));
                    items.insert(k.clone(), cs);
                }
                Self::Object { items }
            }
            Value::Array(arr) => {
                let mut items = CollectionStats::default();
                for item in arr {
                    items.merge_value(TypeStats::new(item));
                }
                Self::Array {
                    example_len: arr.len(),
                    min_len: arr.len(),
                    max_len: arr.len(),
                    items: Box::new(items),
                }
            }
        }
    }

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

    let value: Value = serde_json::from_reader(reader).unwrap_or_else(|e| {
        eprintln!("{} invalid JSON: {}", "error:".red().bold(), e);
        std::process::exit(1);
    });

    let stats = TypeStats::new(&value);
    print::print_root(&stats, &args);
}
