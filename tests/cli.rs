use std::io::Write;
use std::process::{Command, Stdio};

fn jpeek(json: &str) -> String {
    jpeek_with_args(json, &[])
}

fn jpeek_with_args(json: &str, args: &[&str]) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_jpeek"))
        .args(args)
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to run jpeek");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(json.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "jpeek failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    strip_ansi(&String::from_utf8_lossy(&output.stdout))
}

fn strip_ansi(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // skip until 'm'
            while let Some(&next) = chars.peek() {
                chars.next();
                if next == 'm' {
                    break;
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

// --- Scalar roots ---

#[test]
fn root_string() {
    let out = jpeek(r#""hello""#);
    assert_eq!(out.trim(), r#"[root]: str = "hello""#);
}

#[test]
fn root_number() {
    let out = jpeek("42");
    assert_eq!(out.trim(), "[root]: int = 42");
}

#[test]
fn root_float() {
    let out = jpeek("3.14");
    assert_eq!(out.trim(), "[root]: float = 3.14");
}

#[test]
fn root_bool() {
    let out = jpeek("true");
    assert_eq!(out.trim(), "[root]: bool = true");
}

#[test]
fn root_null() {
    let out = jpeek("null");
    assert_eq!(out.trim(), "[root]: null");
}

// --- Simple objects ---

#[test]
fn simple_object() {
    let out = jpeek(r#"{"name": "Alice", "age": 30}"#);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "[root]: obj");
    assert_eq!(lines[1], r#"├── age: int = 30"#);
    assert_eq!(lines[2], r#"└── name: str = "Alice""#);
}

#[test]
fn nested_object() {
    let out = jpeek(r#"{"user": {"name": "Bob", "active": true}}"#);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "[root]: obj");
    assert_eq!(lines[1], "└── user: obj");
    assert_eq!(lines[2], "    ├── active: bool = true");
    assert_eq!(lines[3], r#"    └── name: str = "Bob""#);
}

// --- Arrays ---

#[test]
fn array_of_ints() {
    let out = jpeek("[1, 2, 3]");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "[root]: arr len = 3");
    assert_eq!(lines[1], "└── [values]: int = 1  (1 - 3)");
}

#[test]
fn array_of_objects() {
    let out = jpeek(r#"[{"a": 1}, {"a": 2}]"#);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "[root]: arr len = 2");
    assert_eq!(lines[1], "└── [values]: obj");
    assert_eq!(lines[2], "    └── a: int = 1  (1 - 2)");
}

#[test]
fn empty_array() {
    let out = jpeek("[]");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "[root]: arr len = 0");
    assert_eq!(lines.len(), 1);
}

// --- Unions ---

#[test]
fn union_in_array() {
    let out = jpeek(r#"[1, "two", null]"#);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "[root]: arr len = 3");
    assert_eq!(lines[1], "└── [values]: str | int | null");
    assert_eq!(lines[2], r#"    ├── [option]: str = "two""#);
    assert_eq!(lines[3], "    ├── [option]: int = 1");
    assert_eq!(lines[4], "    └── [option]: null");
}

#[test]
fn union_field_in_object_array() {
    let out = jpeek(r#"[{"a": 1, "b": "hello"}, {"a": "two", "b": null}]"#);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "[root]: arr len = 2");
    assert_eq!(lines[1], "└── [values]: obj");
    assert_eq!(lines[2], "    ├── a: str | int");
    assert_eq!(lines[3], r#"    │   ├── [option]: str = "two""#);
    assert_eq!(lines[4], "    │   └── [option]: int = 1");
    assert_eq!(lines[5], "    └── b: str | null");
    assert_eq!(lines[6], r#"        ├── [option]: str = "hello""#);
    assert_eq!(lines[7], "        └── [option]: null cnt = 1  (0 - 1)");
}

// --- Ranges ---

#[test]
fn string_range() {
    let out = jpeek(r#"[{"s": "apple"}, {"s": "cherry"}, {"s": "banana"}]"#);
    let lines: Vec<&str> = out.lines().collect();
    // example is "apple" (first seen), range is apple..cherry
    assert!(
        lines[2].contains(r#""apple""#),
        "expected apple example in: {}",
        lines[2]
    );
    assert!(
        lines[2].contains(r#""cherry""#),
        "expected cherry in range in: {}",
        lines[2]
    );
}

#[test]
fn number_range() {
    let out = jpeek(r#"[{"n": 10}, {"n": 5}, {"n": 20}]"#);
    let lines: Vec<&str> = out.lines().collect();
    assert!(
        lines[2].contains("(5 - 20)"),
        "expected range in: {}",
        lines[2]
    );
}

#[test]
fn bool_range() {
    let out = jpeek(r#"[{"b": true}, {"b": false}]"#);
    let lines: Vec<&str> = out.lines().collect();
    assert!(
        lines[2].contains("(false - true)"),
        "expected bool range in: {}",
        lines[2]
    );
}

#[test]
fn single_bool_no_range() {
    let out = jpeek(r#"[{"b": true}, {"b": true}]"#);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[2], "    └── b: bool = true");
}

// --- Array length ranges ---

#[test]
fn varying_array_lengths() {
    let out = jpeek(r#"[{"items": [1, 2]}, {"items": [1, 2, 3, 4]}]"#);
    let lines: Vec<&str> = out.lines().collect();
    // items should show len range (2 - 4)
    assert!(
        lines[2].contains("(2 - 4)"),
        "expected array len range in: {}",
        lines[2]
    );
}

// --- Null in union (sorted last) ---

#[test]
fn null_sorted_last_in_union_summary() {
    let out = jpeek(r#"[{"a": null}, {"a": 1}, {"a": "x"}]"#);
    let lines: Vec<&str> = out.lines().collect();
    // union summary should have null last
    assert!(
        lines[2].contains("str | int | null"),
        "expected null last in: {}",
        lines[2]
    );
}

// --- Truncation ---

#[test]
fn string_truncation() {
    let long = "a]".repeat(20); // 40 chars
    let json = format!(r#"{{"s": "{}"}}"#, long);
    let out = jpeek_with_args(&json, &["-l", "10"]);
    let lines: Vec<&str> = out.lines().collect();
    assert!(
        lines[1].contains("..."),
        "expected truncation in: {}",
        lines[1]
    );
}

// --- Nested arrays ---

#[test]
fn nested_array() {
    let out = jpeek(r#"{"matrix": [[1, 2], [3, 4]]}"#);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "[root]: obj");
    assert!(lines[1].contains("matrix:"), "expected matrix field");
    assert!(lines[1].contains("arr"), "expected arr type");
}

// --- Float detection ---

#[test]
fn float_detection() {
    let out = jpeek(r#"[{"x": 1.5}, {"x": 2.5}]"#);
    let lines: Vec<&str> = out.lines().collect();
    assert!(
        lines[2].contains("float"),
        "expected float in: {}",
        lines[2]
    );
}

#[test]
fn int_stays_int() {
    let out = jpeek(r#"[{"x": 1}, {"x": 2}]"#);
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines[2].contains("int"), "expected int in: {}", lines[2]);
}

// --- Deep nesting ---

#[test]
fn deeply_nested() {
    let out = jpeek(r#"{"a": {"b": {"c": {"d": 42}}}}"#);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "[root]: obj");
    assert_eq!(lines[1], "└── a: obj");
    assert_eq!(lines[2], "    └── b: obj");
    assert_eq!(lines[3], "        └── c: obj");
    assert_eq!(lines[4], "            └── d: int = 42");
}

// --- Object fields appear in sorted order (BTreeMap) ---

#[test]
fn fields_sorted() {
    let out = jpeek(r#"{"zebra": 1, "apple": 2, "mango": 3}"#);
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines[1].contains("apple"));
    assert!(lines[2].contains("mango"));
    assert!(lines[3].contains("zebra"));
}

// --- Optional field detection (undefined) ---

#[test]
fn objects_with_varying_fields() {
    let out = jpeek(r#"[{"a": 1}, {"a": 2, "b": "x"}]"#);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "[root]: arr len = 2");
    assert_eq!(lines[1], "└── [values]: obj");
    assert!(lines[2].contains("a: int"), "a should be required: {}", lines[2]);
    assert!(
        lines[3].contains("b: str | undefined"),
        "b should be optional: {}",
        lines[3]
    );
}

#[test]
fn all_fields_missing_from_some_object() {
    let out = jpeek(r#"[{"a": 1}, {"b": 2}]"#);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "[root]: arr len = 2");
    assert_eq!(lines[1], "└── [values]: obj");
    assert!(
        lines[2].contains("a: int | undefined"),
        "a should be optional: {}",
        lines[2]
    );
    assert!(
        lines[5].contains("b: int | undefined"),
        "b should be optional: {}",
        lines[5]
    );
}

#[test]
fn no_undefined_for_single_object() {
    let out = jpeek(r#"{"a": 1, "b": 2}"#);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[1], "├── a: int = 1");
    assert_eq!(lines[2], "└── b: int = 2");
}

#[test]
fn no_undefined_when_all_objects_have_field() {
    let out = jpeek(r#"[{"a": 1}, {"a": 2}]"#);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[2], "    └── a: int = 1  (1 - 2)");
}

#[test]
fn undefined_with_union_type() {
    // a is present in both (required), b has mixed types + missing (optional)
    let out = jpeek(r#"[{"a": 1, "b": "x"}, {"a": 2, "b": 3}, {"a": 3}]"#);
    let lines: Vec<&str> = out.lines().collect();
    assert!(
        lines[2].contains("a: int"),
        "a should be plain int: {}",
        lines[2]
    );
    assert!(!lines[2].contains("undefined"), "a should not be optional: {}", lines[2]);
    // b should be str | int | undefined
    let b_line = lines.iter().find(|l| l.contains("b:")).unwrap();
    assert!(
        b_line.contains("str") && b_line.contains("int") && b_line.contains("undefined"),
        "b should be str | int | undefined: {}",
        b_line
    );
}

#[test]
fn undefined_sorted_last_in_union() {
    let out = jpeek(r#"[{"a": 1}, {"b": 2}]"#);
    let lines: Vec<&str> = out.lines().collect();
    // undefined should come after the real type
    assert!(
        lines[2].contains("int | undefined"),
        "undefined should be last: {}",
        lines[2]
    );
}

#[test]
fn undefined_after_null_in_union() {
    let out = jpeek(r#"[{"a": null}, {"b": 1}]"#);
    let lines: Vec<&str> = out.lines().collect();
    let a_line = lines.iter().find(|l| l.contains("a:")).unwrap();
    assert!(
        a_line.contains("null | undefined"),
        "undefined should be after null: {}",
        a_line
    );
}

#[test]
fn undefined_expands_as_option() {
    let out = jpeek(r#"[{"a": 1}, {"b": 2}]"#);
    let lines: Vec<&str> = out.lines().collect();
    // a: int | undefined should expand to show [option] nodes
    assert!(
        lines[3].contains("[option]: int"),
        "expected int option: {}",
        lines[3]
    );
    assert!(
        lines[4].contains("[option]: undefined"),
        "expected undefined option: {}",
        lines[4]
    );
}
