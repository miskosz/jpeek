# jpeek

Peek at JSON structure — types, examples, and value ranges at a glance.

## Installation

Requires [Rust](https://rustup.rs/).

```sh
git clone https://github.com/michal/jpeek
cd jpeek
cargo install --path .
```

This builds and installs the `jpeek` binary to `~/.cargo/bin/`, which should already be on your `PATH` after a standard Rust installation.

## Usage

```sh
# Analyze a file
jpeek data.json

# Pipe from stdin
cat data.json | jpeek
curl -s https://api.example.com/data | jpeek

# Limit displayed string length (default: 25)
jpeek data.json --max-len 40
```
