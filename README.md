# Krust Sitter

**Krust Sitter is a maintained fork of [rust-sitter](https://github.com/hydro-project/rust-sitter), developed and maintained by wellknown.id ltd.**

Repository: <https://github.com/wellknown-id/krust-sitter>

## Development hooks

This repository uses [`lefthook`](https://github.com/evilmartians/lefthook) for local git hooks.

Install it however you prefer, then enable the hooks:

```sh
cargo install --locked lefthook
lefthook install
```

The pre-commit hook runs:

```sh
cargo clippy --workspace --all-targets -- -D warnings
```

Krust Sitter makes it easy to create efficient parsers in Rust by leveraging the [Tree Sitter](https://tree-sitter.github.io/tree-sitter/) parser generator. With Krust Sitter, you can define your entire grammar with annotations on idiomatic Rust code, and let macros generate the parser and type-safe bindings for you!

## Installation

First, add Rust/Tree Sitter to your `Cargo.toml`:

```toml
[dependencies]
krust-sitter = { git = "https://github.com/wellknown-id/krust-sitter" }

[build-dependencies]
krust-sitter-tool = { git = "https://github.com/wellknown-id/krust-sitter" }
```

The first step is to configure your `build.rs` to compile and link the generated Tree Sitter parser:

```rust
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=src");
    // Path to the file containing your grammar and any submodules.
    krust_sitter_tool::build_parser("src/grammar/mod.rs");
}
```

## Defining a Grammar

Now that we have Krust Sitter added to our project, we can define our grammar. Krust Sitter grammars are defined in Rust modules. First, we create a module file for the grammar in `src/grammar/mod.rs`. Note, this can be any module, however,
due to various quirks with the build system it is required that you have one grammar per module, and all types
in the grammar are defined within it, or a submodule of the module.

Then, inside the module, we can define individual AST nodes. For this simple example, we'll define an expression that can be used in a mathematical expression. Note that we annotate this type as `#[language]` to indicate that it is the root AST type.

```rust
// in ./src/grammar/mod.rs
use krust_sitter::Rule;
#[derive(Rule)]
#[language]
pub enum Expr {
    Number(u32),
    Add(Box<Expr>, Box<Expr>)
}
```

Now that we have the type defined, we must annotate the enum variants to describe how to identify them in the text being parsed. First, we can apply `leaf` to use a regular expression to match digits corresponding to a number.
The value will try to extract the value using a default extraction for the type. For numeric types, this
defaults to `FromStr`. You can specify an alternate function using `#[with]`.

```rust
Number(
    #[leaf(re(r"\d+"))]
    u32,
)
```

For the `Add` variant, things are a bit more complicated. First, we add an extra field corresponding to the `+` that must sit between the two sub-expressions. This can be achieved with `text` or `leaf`, which instructs the parser to match a specific string.

```rust
Add(
    Box<Expr>,
    #[text("+")] (),
    Box<Expr>,
)
```

If we try to compile this grammar, however, we will see an error due to conflicting parse trees for expressions like `1 + 2 + 3`, which could be parsed as `(1 + 2) + 3` or `1 + (2 + 3)`. We want the former, so we can add a further annotation specifying that we want left-associativity for this rule.

```rust
#[prec_left(1)]
Add(
    Box<Expr>,
    #[text("+")] (),
    Box<Expr>,
)
```

All together, our grammar looks like this:

```rust
use krust_sitter::Rule;
#[derive(Rule)]
#[language]
pub enum Expr {
    Number(
        #[leaf(re(r"\d+"))]
        u32,
    ),
    #[prec_left(1)]
    Add(
        Box<Expr>,
        #[text("+")] (),
        Box<Expr>,
    )
}
```

We can then parse text using this grammar:

```rust
dbg!(grammar::Expr::parse("1+2+3").into_result());
/*
grammar::Expr::parse("1+2+3").into_result() = Ok(Add(
    Add(
        Number(
            1,
        ),
        (),
        Number(
            2,
        ),
    ),
    (),
    Number(
        3,
    ),
))
*/
```

## Type Annotations

Krust Sitter supports a number of annotations that can be applied to type and fields in your grammar. These annotations can be used to control how the parser behaves, and how the resulting AST is constructed.

### `#[language]`

This annotation marks the entrypoint for parsing, and determines which AST type will be returned from parsing. Only one type in the grammar can be marked as the entrypoint.

```rust
#[derive(Rule)]
#[language]
struct Code {
    ...
}
```

### `#[extras(...)]`

This annotation can be used on the `#[language]` rule to specify a list of extras. These extras are specified
using the same DSL as `#[leaf(...)]` and `#[text(...)]`. These rules are inserted to the `extras` array in the
grammar.

```rust
#[derive(Rule)]
#[language]
#[extras(
    re(r"\s") // allows whitespace in the grammar.
)]
struct Code {
    ...
}
```

## Field Annotations

### `#[leaf(...)]` and `#[text(...)]`

The `#[leaf(...)]` annotation can be used to define a leaf node in the AST.
`#[text(...)]` is similar, but it does not create a named node in the grammar and cannot be
extracted. It must always be assigned to `()`.

`leaf` and `text` take an input that looks like the [tree sitter
DSL](https://tree-sitter.github.io/tree-sitter/creating-parsers/2-the-grammar-dsl.html). The supported rules
currently are:

- `choice`
- `optional`
- `seq`
- `re` or `pattern` to specify a regular expression
- literal text

Others can be added in the future as needed.

`leaf` can either be applied to a field in a struct / enum variant (as seen above), or directly on a type with no fields:

```rust
#[derive(Rule)]
#[leaf("9")]
struct BigDigit;

#[derive(Rule)]
enum SmallDigit {
    #[leaf("0")]
    Zero,
    #[leaf("1")]
    One,
}
```

### `#[prec(...)]` / `#[prec_left(...)]` / `#[prec_right(...)]` / `#[prec_dynamic(...)]`

This annotation can be used to define a non/left/right-associative operator. This annotation takes a single parameter, which is the precedence level of the operator (higher binds more tightly).

### `#[immediate]`

Usually, whitespace is optional before each token. This attribute means that the token will only match if there is no whitespace.

### `#[skip(...)]`

This annotation can be used to define a field that does not correspond to anything in the input string, such as some metadata. This annotation takes a single parameter, which is the value that should be used to populate that field at runtime.

### `#[word]`

This annotation marks the field as a Tree Sitter [word](https://tree-sitter.github.io/tree-sitter/creating-parsers#keywords), which is useful when handling errors involving keywords. Like `#[extras]`, the `#[word]` is specified on the `#[language]` implementation:

```rust
#[derive(Debug, Rule)]
#[language]
#[word(Ident)]
pub struct Language {
    // ...
}

#[derive(Rule)]
#[leaf(re(r"[a-zA-Z_]+"))]
pub struct Ident;
```

## Partial AST and Errors

Krust Sitter, like tree-sitter, can produce a partial AST along with its errors. Calling `Language::parse` will
produce a `ParseResult` object which includes as much of the AST as it was able to extract, as well as a `Vec`
of all of the parsing errors encountered. This is useful for language servers and other contexts which can
make use of a partial AST. Currently this may not produce the _maximal_ AST, but this may be possible
in the future.

## Credits

See [`CREDITS.md`](./CREDITS.md) for attribution to the original `rust-sitter` project and the creators of this fork.

## Special Types

Krust Sitter has a few special types that can be used to define more complex grammars.

### `Vec<T>`

To parse repeating structures, you can use a `Vec<T>` to parse a list of `T`s. Note that the `Vec<T>` type **cannot** be wrapped in another `Vec` (create additional structs if this is necessary). There are two special attributes that can be applied to a `Vec` field to control the parsing behavior.

The `#[sep_by(...)]` attribute can be used to specify a separator between elements of the
list. This is parsed in the same way as `text` and `leaf` and therefore supports all of the listed tree-sitter
grammar above.

```rust
pub struct CommaSeparatedExprs {
    #[sep_by(",")]
    numbers: Vec<Expr>,
}
```

The `#[repeat1]` can be used to specify that the list must contain at least, or you can use `#[sep_by1(...)]

```rust
pub struct CommaSeparatedExprs {
    #[repeat1]
    #[sep_by(",")]
    // Or just use #[sep_by1(",")]
    numbers: Vec<Expr>,
}
```

### `Option<T>`

To parse optional structures, you can use an `Option<T>` to parse a single `T` or nothing. Like `Vec`, the `Option<T>` type **cannot** be wrapped in another `Option` (create additional structs if this is necessary). For example, we can make the list elements in the previous example optional so we can parse strings like `1,,2`:

```rust
pub struct CommaSeparatedExprs {
    #[sep_by1(",")]
    numbers: Vec<Option<Expr>>,
}
```

### `krust_sitter::Spanned<T>`

When using Krust Sitter to power diagnostic tools, it can be helpful to access spans marking the sections of text corresponding to a parsed node. To do this, you can use the `Spanned<T>` type, which captures the underlying parsed `T` and a pair of indices for the start (inclusive) and end (exclusive) of the corresponding substring. `Spanned` types can be used anywhere, and do not affect the parsing logic. For example, we could capture the spans of the expressions in our previous example:

```rust
pub struct CommaSeparatedExprs {
    #[sep_by1(",")]
    numbers: Vec<Option<Spanned<Expr>>>,
}
```

### `Box<T>`

Boxes are automatically constructed around the inner type when parsing, but Krust Sitter doesn't do anything extra beyond that.

## TextMate Grammar Generation

Krust Sitter can also generate `.tmLanguage.json` grammars for legacy syntax highlighters (VS Code, Sublime Text, etc.) from the same grammar definitions used for Tree Sitter parsing.

### Setup

Add `TextMateBuilder` to your `build.rs` alongside the existing parser generation:

```rust
fn main() {
    println!("cargo:rerun-if-changed=src");

    // Generate Tree Sitter parser (existing)
    krust_sitter_tool::build_parser("src/grammar/mod.rs");

    // Generate TextMate grammar (new)
    if let Some(textmate) = krust_sitter_tool::TextMateBuilder::default()
        .scope_name("my-language")
        .build("src/grammar/mod.rs")
    {
        let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
        std::fs::write(
            out.join("my-language.tmLanguage.json"),
            serde_json::to_string_pretty(&textmate).unwrap(),
        ).unwrap();
    }
}
```

### How It Works

The generator walks the same `Grammar` IR used for Tree Sitter and automatically categorises tokens:

| Grammar Annotation                      | TextMate Scope     |
| --------------------------------------- | ------------------ |
| `#[leaf("let")]` / `#[text("keyword")]` | `keyword.control`  |
| `#[leaf("+")]` / `#[text(";")]`         | `keyword.operator` |
| `#[leaf(re(r"\d+"))]`                   | `constant.numeric` |
| `#[leaf(re(r"[a-zA-Z_]\w*"))]`          | `variable.other`   |

Bracket-like delimiter pairs (`()`, `{}`, `[]`) in struct sequences are detected and emitted as TextMate `begin`/`end` rules.

### Custom Scope Name

Use `.scope_name()` to set the TextMate `scopeName` (defaults to the grammar name):

```rust
TextMateBuilder::default()
    .scope_name("karu")  // produces scopeName: "source.karu"
    .build("src/grammar/mod.rs");
```
