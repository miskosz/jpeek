use colored::Colorize;
use std::collections::BTreeMap;

use crate::{Args, CollectionStats, TypeStats};

pub(crate) fn print_root(stats: &TypeStats, args: &Args) {
    match stats {
        TypeStats::Object { items } => {
            println!("{}: {}", "[root]".bright_magenta(), "obj".bright_yellow());
            print_object_fields(items, &[], args);
        }
        TypeStats::Array {
            example_len,
            min_len,
            max_len,
            items,
        } => {
            println!(
                "{}: {}",
                "[root]".bright_magenta(),
                format_array_len(*example_len, *min_len, *max_len)
            );
            print_field_stats(items, &[], args, true);
        }
        _ => {
            let (ex, _rng) = stats.format_value(args.max_len);
            let type_name = stats.display_name().bright_yellow();
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

fn summarize_field_types(cs: &CollectionStats) -> String {
    let mut names: Vec<&str> = cs.types.values().map(|v| v.display_name()).collect();
    if let Some(pos) = names.iter().position(|&n| n == "null") {
        let null = names.remove(pos);
        names.push(null);
    }
    if let Some(pos) = names.iter().position(|&n| n == "undefined") {
        let undef = names.remove(pos);
        names.push(undef);
    }
    names.join(" | ")
}

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
    let mut line = format!(
        "{}{}: {}",
        prefix,
        color_label(label),
        type_str.bright_yellow()
    );
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
        s.push_str(&format!(
            "  {}",
            format!("({} - {})", min_len, max_len).dimmed()
        ));
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
        TypeStats::Null { .. } | TypeStats::Undefined { .. } => {
            let (ex, rng) = stats.format_value(args.max_len);
            if ex.is_empty() {
                print_entry(ancestors, is_last, label, stats.display_name(), "", "");
            } else {
                let prefix = tree_prefix(ancestors, is_last);
                let mut line = format!(
                    "{}{}: {} {} = {}",
                    prefix,
                    color_label(label),
                    stats.display_name().bright_yellow(),
                    "cnt".bright_white(),
                    ex.bright_green()
                );
                if !rng.is_empty() {
                    line.push_str(&format!("  {}", rng.dimmed()));
                }
                println!("{}", line);
            }
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
