use clap::Parser;
use colored::Colorize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader, Read};

#[derive(Parser)]
#[command(name = "jpeek", about = "Peek at JSON structure — types, examples, value ranges at a glance")]
struct Args {
    /// JSON file to analyze (reads stdin if omitted)
    file: Option<String>,

    /// Max length for displayed string values
    #[arg(short = 'l', long, default_value_t = 25)]
    max_len: usize,
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
        merged: BTreeMap<String, FieldStats>,
    },
    Array {
        example_len: usize,
        min_len: usize,
        max_len: usize,
        item_stats: Box<FieldStats>,
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
                let mut merged = BTreeMap::new();
                for (k, v) in map {
                    let mut fs = FieldStats::default();
                    fs.merge_value(v, args);
                    merged.insert(k.clone(), fs);
                }
                Self::Object { merged }
            }
            Value::Array(arr) => {
                let mut item_stats = FieldStats::default();
                for item in arr {
                    item_stats.merge_value(item, args);
                }
                Self::Array {
                    example_len: arr.len(),
                    min_len: arr.len(),
                    max_len: arr.len(),
                    item_stats: Box::new(item_stats),
                }
            }
        }
    }

    fn merge(&mut self, val: &Value, args: &Args) {
        match (self, val) {
            (Self::String { min_val, max_val, .. }, Value::String(s)) => {
                if s < min_val { *min_val = s.clone(); }
                if s > max_val { *max_val = s.clone(); }
            }
            (Self::Number { min, max, is_float, .. }, Value::Number(n)) => {
                let f = n.as_f64().unwrap_or(0.0);
                *min = min.min(f);
                *max = max.max(f);
                if n.is_f64() && f.fract() != 0.0 { *is_float = true; }
            }
            (Self::Bool { has_true, has_false, .. }, Value::Bool(b)) => {
                if *b { *has_true = true; } else { *has_false = true; }
            }
            (Self::Object { merged }, Value::Object(map)) => {
                for (k, v) in map {
                    merged.entry(k.clone()).or_default().merge_value(v, args);
                }
            }
            (Self::Array { min_len, max_len, item_stats, .. }, Value::Array(arr)) => {
                *min_len = (*min_len).min(arr.len());
                *max_len = (*max_len).max(arr.len());
                for item in arr { item_stats.merge_value(item, args); }
            }
            _ => {}
        }
    }

    fn type_name(&self) -> &'static str {
        match self {
            Self::String { .. } => "string",
            Self::Number { is_float, .. } => if *is_float { "float" } else { "int" },
            Self::Bool { .. } => "bool",
            Self::Null => "null",
            Self::Object { .. } => "object",
            Self::Array { .. } => "array",
        }
    }

    /// Returns (example, optional_range) for leaf types
    fn format_value(&self, max_len: usize) -> (String, String) {
        match self {
            Self::String { example, min_val, max_val } => {
                let ex = format!("\"{}\"", truncate(example, max_len));
                if min_val == max_val {
                    (ex, String::new())
                } else {
                    (ex, format!("(\"{}\" - \"{}\")", truncate(min_val, max_len), truncate(max_val, max_len)))
                }
            }
            Self::Number { example, min, max, is_float } => {
                if (max - min).abs() < f64::EPSILON {
                    (format_number(*example, *is_float), String::new())
                } else {
                    (format_number(*example, *is_float),
                     format!("({} - {})", format_number(*min, *is_float), format_number(*max, *is_float)))
                }
            }
            Self::Bool { example, has_true, has_false } => {
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
struct FieldStats {
    types: Vec<(&'static str, TypeStats)>,
}

impl FieldStats {
    fn merge_value(&mut self, val: &Value, args: &Args) {
        let type_name = type_label(val);

        // Merge int into float (int is a subset of float)
        let pos = if type_name == "int" {
            self.types.iter().position(|(n, _)| *n == "int")
                .or_else(|| self.types.iter().position(|(n, _)| *n == "float"))
        } else if type_name == "float" {
            if let Some(pos) = self.types.iter().position(|(n, _)| *n == "int") {
                self.types[pos].0 = "float";
                if let TypeStats::Number { is_float, .. } = &mut self.types[pos].1 {
                    *is_float = true;
                }
                Some(pos)
            } else {
                self.types.iter().position(|(n, _)| *n == "float")
            }
        } else {
            self.types.iter().position(|(n, _)| *n == type_name)
        };

        if let Some(pos) = pos {
            self.types[pos].1.merge(val, args);
        } else {
            self.types.push((type_name, TypeStats::new(val, args)));
        }
    }
}

fn type_label(val: &Value) -> &'static str {
    match val {
        Value::String(_) => "string",
        Value::Number(n) => {
            if n.is_f64() && n.as_f64().map_or(false, |f| f.fract() != 0.0) {
                "float"
            } else {
                "int"
            }
        }
        Value::Bool(_) => "bool",
        Value::Null => "null",
        Value::Object(_) => "object",
        Value::Array(_) => "array",
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

    let mut fs = FieldStats::default();
    fs.merge_value(&value, &args);

    match &value {
        Value::Object(_) => {
            println!("{}: {}", "[root]".bright_magenta(), "obj".bright_yellow());
            print_field_stats(&fs, &[], &args, false);
        }
        Value::Array(arr) => {
            if let Some((_, TypeStats::Array { min_len, max_len, item_stats, .. })) = fs.types.first() {
                print_root_array(arr.len(), *min_len, *max_len);
                print_field_stats(item_stats, &[], &args, true);
            } else {
                print_root_array(arr.len(), arr.len(), arr.len());
            }
        }
        _ => {
            println!("{}: {}", "[root]".bright_magenta(), display_type_name(type_label(&value)).bright_yellow());
        }
    }
}

fn print_root_array(example_len: usize, min_len: usize, max_len: usize) {
    let colored_example = format!("{}", example_len).bright_green();
    if min_len == max_len {
        println!("{}: {} {} = {}", "[root]".bright_magenta(), "arr".bright_yellow(), "len".bright_white(), colored_example);
    } else {
        let range = format!("({} - {})", min_len, max_len);
        println!("{}: {} {} = {}  {}", "[root]".bright_magenta(), "arr".bright_yellow(), "len".bright_white(), colored_example, range.dimmed());
    }
}

// --- Display helpers ---

fn display_type_name(internal: &str) -> &str {
    match internal {
        "string" => "str",
        "object" => "obj",
        "array" => "arr",
        _ => internal,
    }
}

fn format_number(n: f64, is_float: bool) -> String {
    if is_float { format!("{}", n) } else { format!("{}", n as i64) }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(max).collect::<String>())
    }
}

fn summarize_field_types(fs: &FieldStats) -> String {
    let mut names: Vec<&str> = fs.types.iter().map(|(n, _)| display_type_name(n)).collect();
    if names.contains(&"float") && names.contains(&"int") {
        names.retain(|&n| n != "int");
    }
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

fn print_entry(ancestors: &[bool], is_last: bool, label: &str, type_str: &str, example: &str, range: &str) {
    let prefix = tree_prefix(ancestors, is_last);
    let colored_type = display_type_name(type_str).bright_yellow();

    if !example.is_empty() && !range.is_empty() {
        println!("{}{}: {} = {}  {}", prefix, color_label(label), colored_type, example.bright_green(), range.dimmed());
    } else if !example.is_empty() {
        println!("{}{}: {} = {}", prefix, color_label(label), colored_type, example.bright_green());
    } else {
        println!("{}{}: {}", prefix, color_label(label), colored_type);
    }
}

fn print_array_entry(ancestors: &[bool], is_last: bool, label: &str, example_len: usize, min_len: usize, max_len: usize) {
    let prefix = tree_prefix(ancestors, is_last);
    let colored_example = format!("{}", example_len).bright_green();

    if min_len == max_len {
        println!("{}{}: {} {} = {}", prefix, color_label(label),
            "arr".bright_yellow(), "len".bright_white(), colored_example);
    } else {
        let range = format!("({} - {})", min_len, max_len);
        println!("{}{}: {} {} = {}  {}", prefix, color_label(label),
            "arr".bright_yellow(), "len".bright_white(), colored_example, range.dimmed());
    }
}

// --- Recursive printing ---

fn print_field_stats(fs: &FieldStats, ancestors: &[bool], args: &Args, in_array: bool) {
    let is_union = fs.types.len() > 1;

    for (i, (_type_name, type_stats)) in fs.types.iter().enumerate() {
        let is_last = i == fs.types.len() - 1;

        if is_union {
            print_stats_node(type_stats, ancestors, is_last, args, "[option]");
        } else if in_array {
            print_stats_node(type_stats, ancestors, true, args, "[values]");
        } else if let TypeStats::Object { merged } = type_stats {
            print_object_fields(merged, ancestors, args);
        } else {
            print_stats_node(type_stats, ancestors, true, args, "[values]");
        }
    }
}

fn print_stats_node(stats: &TypeStats, ancestors: &[bool], is_last: bool, args: &Args, label: &str) {
    match stats {
        TypeStats::Object { merged } => {
            print_entry(ancestors, is_last, label, "object", "", "");
            let mut child = ancestors.to_vec();
            child.push(is_last);
            print_object_fields(merged, &child, args);
        }
        TypeStats::Array { example_len, min_len, max_len, item_stats } => {
            print_array_entry(ancestors, is_last, label, *example_len, *min_len, *max_len);
            let mut child = ancestors.to_vec();
            child.push(is_last);
            print_field_stats(item_stats, &child, args, true);
        }
        _ => {
            let (ex, rng) = stats.format_value(args.max_len);
            print_entry(ancestors, is_last, label, stats.type_name(), &ex, &rng);
        }
    }
}

fn print_object_fields(merged: &BTreeMap<String, FieldStats>, ancestors: &[bool], args: &Args) {
    let keys: Vec<_> = merged.iter().collect();
    let len = keys.len();

    for (i, (key, field_stats)) in keys.iter().enumerate() {
        let is_last = i == len - 1;

        if field_stats.types.len() > 1 {
            let type_summary = summarize_field_types(field_stats);
            print_entry(ancestors, is_last, key, &type_summary, "", "");
            let mut child = ancestors.to_vec();
            child.push(is_last);
            print_field_stats(field_stats, &child, args, true);
        } else if let Some((_tn, stats)) = field_stats.types.first() {
            print_field_node(stats, ancestors, is_last, args, key);
        }
    }
}

/// Print a named field (object key). Inlines object children directly.
fn print_field_node(stats: &TypeStats, ancestors: &[bool], is_last: bool, args: &Args, key: &str) {
    match stats {
        TypeStats::Object { merged } => {
            print_entry(ancestors, is_last, key, "object", "", "");
            let mut child = ancestors.to_vec();
            child.push(is_last);
            print_object_fields(merged, &child, args);
        }
        TypeStats::Array { example_len, min_len, max_len, item_stats } => {
            print_array_entry(ancestors, is_last, key, *example_len, *min_len, *max_len);
            let mut child = ancestors.to_vec();
            child.push(is_last);
            print_field_stats(item_stats, &child, args, true);
        }
        _ => {
            let (ex, rng) = stats.format_value(args.max_len);
            print_entry(ancestors, is_last, key, stats.type_name(), &ex, &rng);
        }
    }
}
