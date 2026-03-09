use clap::Parser;
use colored::Colorize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader, Read};

#[derive(Parser)]
#[command(
    name = "jpeek",
    about = "Peek at JSON structure — types, examples, value ranges at a glance"
)]
struct Args {
    /// JSON file to analyze (reads stdin if omitted)
    file: Option<String>,

    /// Max length for displayed string values
    #[arg(short = 'l', long, default_value_t = 25)]
    max_len: usize,
}

/// Discriminant for the JSON type of a field
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum TypeKey {
    String,
    Number,
    Bool,
    Null,
    Object,
    Array,
}

/// Tracks statistics for a single type occurrence of a field
#[derive(Clone, Debug)]
enum TypeStats {
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
    Null,
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
    fn new(val: &Value, args: &Args) -> Self {
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
            Value::Null => Self::Null,
            Value::Object(map) => {
                let mut items = BTreeMap::new();
                for (k, v) in map {
                    let mut cs = CollectionStats::default();
                    cs.merge_value(TypeStats::new(v, args));
                    items.insert(k.clone(), cs);
                }
                Self::Object { items }
            }
            Value::Array(arr) => {
                let mut items = CollectionStats::default();
                for item in arr {
                    items.merge_value(TypeStats::new(item, args));
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
            (Self::Object { items }, Self::Object { items: other_items }) => {
                for (k, v) in other_items {
                    items.entry(k).or_default().merge(v);
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

    fn display_name(&self) -> &'static str {
        match self {
            Self::String { .. } => "str",
            Self::Number { is_float: true, .. } => "float",
            Self::Number {
                is_float: false, ..
            } => "int",
            Self::Bool { .. } => "bool",
            Self::Null => "null",
            Self::Object { .. } => "obj",
            Self::Array { .. } => "arr",
        }
    }

    fn type_key(&self) -> TypeKey {
        match self {
            Self::String { .. } => TypeKey::String,
            Self::Number { .. } => TypeKey::Number,
            Self::Bool { .. } => TypeKey::Bool,
            Self::Null => TypeKey::Null,
            Self::Object { .. } => TypeKey::Object,
            Self::Array { .. } => TypeKey::Array,
        }
    }

    /// Returns (example, optional_range) for leaf types
    fn format_value(&self, max_len: usize) -> (String, String) {
        match self {
            Self::String {
                example,
                min_val,
                max_val,
            } => {
                let ex = format!("\"{}\"", truncate(example, max_len));
                if min_val == max_val {
                    (ex, String::new())
                } else {
                    (
                        ex,
                        format!(
                            "(\"{}\" - \"{}\")",
                            truncate(min_val, max_len),
                            truncate(max_val, max_len)
                        ),
                    )
                }
            }
            Self::Number {
                example,
                min,
                max,
                is_float,
            } => {
                if (max - min).abs() < f64::EPSILON {
                    (format_number(*example, *is_float), String::new())
                } else {
                    (
                        format_number(*example, *is_float),
                        format!(
                            "({} - {})",
                            format_number(*min, *is_float),
                            format_number(*max, *is_float)
                        ),
                    )
                }
            }
            Self::Bool {
                example,
                has_true,
                has_false,
            } => {
                if *has_true && *has_false {
                    (format!("{}", example), "(false - true)".to_string())
                } else if *has_true {
                    ("true".to_string(), String::new())
                } else {
                    ("false".to_string(), String::new())
                }
            }
            _ => (String::new(), String::new()),
        }
    }
}

/// Tracks all type variants seen for a field
#[derive(Clone, Debug, Default)]
struct CollectionStats {
    types: BTreeMap<TypeKey, TypeStats>,
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
        for (_, stats) in other.types {
            self.merge_value(stats);
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

    let mut cs = CollectionStats::default();
    cs.merge_value(TypeStats::new(&value, &args));

    match &value {
        Value::Object(_) => {
            println!("{}: {}", "[root]".bright_magenta(), "obj".bright_yellow());
            print_field_stats(&cs, &[], &args, false);
        }
        Value::Array(arr) => {
            if let Some(TypeStats::Array {
                min_len,
                max_len,
                items,
                ..
            }) = cs.types.get(&TypeKey::Array)
            {
                println!("{}: {}", "[root]".bright_magenta(), format_array_len(arr.len(), *min_len, *max_len));
                print_field_stats(items, &[], &args, true);
            } else {
                println!("{}: {}", "[root]".bright_magenta(), format_array_len(arr.len(), arr.len(), arr.len()));
            }
        }
        _ => {
            let ts = TypeStats::new(&value, &args);
            let (ex, _rng) = ts.format_value(args.max_len);
            let type_name = ts.display_name().bright_yellow();
            if ex.is_empty() {
                println!("{}: {}", "[root]".bright_magenta(), type_name);
            } else {
                println!(
                    "{}: {} = {}",
                    "[root]".bright_magenta(),
                    type_name,
                    ex.bright_green()
                );
            }
        }
    }
}

// --- Display helpers ---

fn format_number(n: f64, is_float: bool) -> String {
    if is_float {
        format!("{}", n)
    } else {
        format!("{}", n as i64)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(max).collect::<String>())
    }
}

fn summarize_field_types(cs: &CollectionStats) -> String {
    let mut names: Vec<&str> = cs.types.values().map(|v| v.display_name()).collect();
    if let Some(pos) = names.iter().position(|&n| n == "null") {
        let null = names.remove(pos);
        names.push(null);
    }
    names.join(" | ")
}

// --- Tree rendering ---

fn tree_prefix(ancestors: &[bool], is_last: bool) -> String {
    let mut s = String::new();
    for &ancestor_is_last in ancestors {
        s.push_str(if ancestor_is_last { "    " } else { "│   " });
    }
    s.push_str(if is_last { "└── " } else { "├── " });
    s
}

fn color_label(label: &str) -> String {
    if label == "[values]" || label == "[option]" {
        label.bright_magenta().to_string()
    } else {
        label.bright_blue().to_string()
    }
}

fn print_entry(
    ancestors: &[bool],
    is_last: bool,
    label: &str,
    type_str: &str,
    example: &str,
    range: &str,
) {
    let prefix = tree_prefix(ancestors, is_last);
    let mut line = format!("{}{}: {}", prefix, color_label(label), type_str.bright_yellow());
    if !example.is_empty() {
        line.push_str(&format!(" = {}", example.bright_green()));
        if !range.is_empty() {
            line.push_str(&format!("  {}", range.dimmed()));
        }
    }
    println!("{}", line);
}

fn format_array_len(example_len: usize, min_len: usize, max_len: usize) -> String {
    let mut s = format!(
        "{} {} = {}",
        "arr".bright_yellow(),
        "len".bright_white(),
        format!("{}", example_len).bright_green()
    );
    if min_len != max_len {
        s.push_str(&format!("  {}", format!("({} - {})", min_len, max_len).dimmed()));
    }
    s
}

fn print_array_entry(
    ancestors: &[bool],
    is_last: bool,
    label: &str,
    example_len: usize,
    min_len: usize,
    max_len: usize,
) {
    let prefix = tree_prefix(ancestors, is_last);
    println!(
        "{}{}: {}",
        prefix,
        color_label(label),
        format_array_len(example_len, min_len, max_len)
    );
}

// --- Recursive printing ---

fn print_field_stats(cs: &CollectionStats, ancestors: &[bool], args: &Args, in_array: bool) {
    let is_union = cs.types.len() > 1;
    let entries: Vec<_> = cs.types.iter().collect();

    if is_union && in_array {
        let type_summary = summarize_field_types(cs);
        print_entry(ancestors, true, "[values]", &type_summary, "", "");
        let mut child = ancestors.to_vec();
        child.push(true);
        for (i, (_key, type_stats)) in entries.iter().enumerate() {
            let is_last = i == entries.len() - 1;
            print_stats_node(type_stats, &child, is_last, args, "[option]");
        }
        return;
    }

    for (i, (_key, type_stats)) in entries.iter().enumerate() {
        let is_last = i == entries.len() - 1;

        if is_union {
            print_stats_node(type_stats, ancestors, is_last, args, "[option]");
        } else if in_array {
            print_stats_node(type_stats, ancestors, true, args, "[values]");
        } else if let TypeStats::Object { items } = type_stats {
            print_object_fields(items, ancestors, args);
        } else {
            print_stats_node(type_stats, ancestors, true, args, "[values]");
        }
    }
}

fn print_stats_node(
    stats: &TypeStats,
    ancestors: &[bool],
    is_last: bool,
    args: &Args,
    label: &str,
) {
    match stats {
        TypeStats::Object { items } => {
            print_entry(ancestors, is_last, label, "obj", "", "");
            let mut child = ancestors.to_vec();
            child.push(is_last);
            print_object_fields(items, &child, args);
        }
        TypeStats::Array {
            example_len,
            min_len,
            max_len,
            items,
        } => {
            print_array_entry(ancestors, is_last, label, *example_len, *min_len, *max_len);
            let mut child = ancestors.to_vec();
            child.push(is_last);
            print_field_stats(items, &child, args, true);
        }
        _ => {
            let (ex, rng) = stats.format_value(args.max_len);
            print_entry(ancestors, is_last, label, stats.display_name(), &ex, &rng);
        }
    }
}

fn print_object_fields(items: &BTreeMap<String, CollectionStats>, ancestors: &[bool], args: &Args) {
    let keys: Vec<_> = items.iter().collect();
    let len = keys.len();

    for (i, (key, field_stats)) in keys.iter().enumerate() {
        let is_last = i == len - 1;

        if field_stats.types.len() > 1 {
            let type_summary = summarize_field_types(field_stats);
            print_entry(ancestors, is_last, key, &type_summary, "", "");
            let mut child = ancestors.to_vec();
            child.push(is_last);
            print_field_stats(field_stats, &child, args, false);
        } else if let Some((_key, stats)) = field_stats.types.iter().next() {
            print_stats_node(stats, ancestors, is_last, args, key);
        }
    }
}

