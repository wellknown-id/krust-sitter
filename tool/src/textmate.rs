//! TextMate grammar generation from the Grammar IR.
//!
//! This module converts a `Grammar` (the same IR used for Tree-Sitter generation)
//! into a `.tmLanguage.json` value suitable for legacy syntax highlighters.

use rust_sitter_types::grammar::{Grammar, RuleDef};
use serde_json::{Value, json};

/// Generate a complete `.tmLanguage.json` value from a Grammar IR.
pub fn generate_textmate(grammar: &Grammar, scope_name: Option<&str>) -> Value {
    let lang_name = &grammar.name;
    let scope = scope_name.unwrap_or_else(|| lang_name.as_str());

    let mut collector = TokenCollector::new(scope);
    // Collect comment patterns from extras.
    collector.collect_extras(&grammar.extras);
    // Walk all rules to collect tokens.
    for (rule_name, rule_def) in &grammar.rules {
        collector.collect_rule(rule_name, rule_def);
    }

    collector.to_textmate_json(lang_name, scope)
}

/// Categorised token for TextMate pattern generation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum TokenCategory {
    Keyword,
    Operator,
    Numeric,
    Identifier,
    String,
}

/// A collected token pattern ready for TextMate emission.
#[derive(Debug, Clone)]
struct CollectedToken {
    pattern: String,
    category: TokenCategory,
}

/// A begin/end pair for TextMate.
#[derive(Debug, Clone)]
struct BeginEndPair {
    begin: String,
    end: String,
    name: String,
}

/// A comment pattern extracted from grammar extras.
#[derive(Debug, Clone)]
enum CommentPattern {
    /// A single-line comment matched by a regex (e.g. `//.*$`).
    Line { pattern: String, scope: String },
    /// A block comment with begin/end markers (e.g. `/* ... */`).
    Block { begin: String, end: String, scope: String },
}

/// Walks the Grammar IR and collects tokens into categories.
struct TokenCollector {
    tokens: Vec<CollectedToken>,
    begin_end_pairs: Vec<BeginEndPair>,
    comment_patterns: Vec<CommentPattern>,
    scope: String,
}

impl TokenCollector {
    fn new(scope: &str) -> Self {
        Self {
            tokens: Vec::new(),
            begin_end_pairs: Vec::new(),
            comment_patterns: Vec::new(),
            scope: scope.to_string(),
        }
    }

    /// Inspect grammar `extras` for comment-like patterns and collect them.
    ///
    /// Recognised heuristics:
    /// - Regex containing `//` → line comment (emitted as `//.*$`)
    /// - Regex containing `/\*` → block comment (emitted as `/* … */`)
    fn collect_extras(&mut self, extras: &[RuleDef]) {
        for extra in extras {
            // Unwrap TOKEN/IMMEDIATE_TOKEN wrappers (e.g. from `token(re(...))`)
            let inner = match extra {
                RuleDef::TOKEN { content } | RuleDef::IMMEDIATE_TOKEN { content } => {
                    content.as_ref()
                }
                other => other,
            };
            if let RuleDef::PATTERN { value, .. } = inner {
                // Skip pure whitespace extras like `\s`
                let trimmed = value.trim();
                if trimmed == r"\s" || trimmed == r"\s+" {
                    continue;
                }

                // Detect line-comment patterns (contain `//`)
                if value.contains("//") {
                    self.comment_patterns.push(CommentPattern::Line {
                        pattern: "//.*$".to_string(),
                        scope: format!("comment.line.double-slash.{}", self.scope),
                    });
                    continue;
                }

                // Detect block-comment patterns (contain `/*`)
                if value.contains("/*") || value.contains(r"/\*") {
                    self.comment_patterns.push(CommentPattern::Block {
                        begin: r"/\*".to_string(),
                        end: r"\*/".to_string(),
                        scope: format!("comment.block.{}", self.scope),
                    });
                    continue;
                }

                // Detect `#`-style line comments
                if value.starts_with('#') {
                    self.comment_patterns.push(CommentPattern::Line {
                        pattern: "#.*$".to_string(),
                        scope: format!("comment.line.number-sign.{}", self.scope),
                    });
                }
            }
        }
    }

    fn collect_rule(&mut self, _rule_name: &str, rule_def: &RuleDef) {
        // Try to detect begin/end pairs from SEQ structures first.
        if let Some(pair) = self.try_extract_begin_end(rule_def) {
            // Deduplicate: only add if we don't already have this begin/end pair.
            if !self
                .begin_end_pairs
                .iter()
                .any(|p| p.begin == pair.begin && p.end == pair.end)
            {
                self.begin_end_pairs.push(pair);
            }
        }
        // Then collect individual tokens recursively.
        self.collect_tokens(rule_def);
    }

    fn collect_tokens(&mut self, rule_def: &RuleDef) {
        match rule_def {
            RuleDef::STRING { value } => {
                let token = CollectedToken {
                    pattern: value.clone(),
                    category: categorise_string(value),
                };
                if !self.tokens.iter().any(|t| t.pattern == token.pattern) {
                    self.tokens.push(token);
                }
            }
            RuleDef::PATTERN { value, .. } => {
                let token = CollectedToken {
                    pattern: value.clone(),
                    category: categorise_pattern(value),
                };
                if !self.tokens.iter().any(|t| t.pattern == token.pattern) {
                    self.tokens.push(token);
                }
            }
            // Recurse through structural wrappers.
            RuleDef::CHOICE { members } | RuleDef::SEQ { members } => {
                for member in members {
                    self.collect_tokens(member);
                }
            }
            RuleDef::REPEAT { content }
            | RuleDef::REPEAT1 { content }
            | RuleDef::PREC { content, .. }
            | RuleDef::PREC_LEFT { content, .. }
            | RuleDef::PREC_RIGHT { content, .. }
            | RuleDef::PREC_DYNAMIC { content, .. }
            | RuleDef::FIELD { content, .. }
            | RuleDef::TOKEN { content }
            | RuleDef::IMMEDIATE_TOKEN { content }
            | RuleDef::ALIAS { content, .. }
            | RuleDef::RESERVED { content, .. } => {
                self.collect_tokens(content);
            }
            // SYMBOL and BLANK contribute no direct tokens.
            RuleDef::SYMBOL { .. } | RuleDef::BLANK => {}
        }
    }

    /// Try to extract a begin/end pair from a SEQ like `"(" ... ")"`.
    fn try_extract_begin_end(&self, rule_def: &RuleDef) -> Option<BeginEndPair> {
        let members = unwrap_to_seq(rule_def)?;
        if members.len() < 2 {
            return None;
        }

        let begin_str = extract_string_value(&members[0])?;
        let end_str = extract_string_value(members.last()?)?;

        // Only generate begin/end for bracket-like delimiters.
        let is_bracket_pair = matches!(
            (begin_str.as_str(), end_str.as_str()),
            ("(", ")") | ("{", "}") | ("[", "]") | ("<", ">")
        );
        if !is_bracket_pair {
            return None;
        }

        Some(BeginEndPair {
            begin: regex_escape(&begin_str),
            end: regex_escape(&end_str),
            name: format!(
                "meta.block.{}.{}",
                begin_str
                    .replace('(', "paren")
                    .replace('{', "brace")
                    .replace('[', "bracket")
                    .replace('<', "angle"),
                self.scope
            ),
        })
    }

    /// Build the final `.tmLanguage.json` output.
    fn to_textmate_json(&self, lang_name: &str, scope: &str) -> Value {
        let mut top_patterns: Vec<Value> = Vec::new();
        let mut repository = serde_json::Map::new();

        // --- Comments (emitted first so they take priority) ---
        if !self.comment_patterns.is_empty() {
            let mut comment_pats: Vec<Value> = Vec::new();
            for cp in &self.comment_patterns {
                match cp {
                    CommentPattern::Line { pattern, scope } => {
                        comment_pats.push(json!({
                            "match": pattern,
                            "name": scope
                        }));
                    }
                    CommentPattern::Block { begin, end, scope } => {
                        comment_pats.push(json!({
                            "begin": begin,
                            "end": end,
                            "name": scope
                        }));
                    }
                }
            }
            repository.insert(
                "comments".to_string(),
                json!({ "patterns": comment_pats }),
            );
            top_patterns.push(json!({ "include": "#comments" }));
        }

        // --- Keywords ---
        let keywords: Vec<&CollectedToken> = self
            .tokens
            .iter()
            .filter(|t| t.category == TokenCategory::Keyword)
            .collect();
        if !keywords.is_empty() {
            let kw_pattern = keywords
                .iter()
                .map(|t| regex_escape(&t.pattern))
                .collect::<Vec<_>>()
                .join("|");
            repository.insert(
                "keywords".to_string(),
                json!({
                    "patterns": [{
                        "match": format!("\\b(?:{kw_pattern})\\b"),
                        "name": format!("keyword.control.{scope}")
                    }]
                }),
            );
            top_patterns.push(json!({ "include": "#keywords" }));
        }

        // --- Operators ---
        let operators: Vec<&CollectedToken> = self
            .tokens
            .iter()
            .filter(|t| t.category == TokenCategory::Operator)
            .collect();
        if !operators.is_empty() {
            let op_pattern = operators
                .iter()
                .map(|t| regex_escape(&t.pattern))
                .collect::<Vec<_>>()
                .join("|");
            repository.insert(
                "operators".to_string(),
                json!({
                    "patterns": [{
                        "match": op_pattern,
                        "name": format!("keyword.operator.{scope}")
                    }]
                }),
            );
            top_patterns.push(json!({ "include": "#operators" }));
        }

        // --- Numeric constants ---
        let numerics: Vec<&CollectedToken> = self
            .tokens
            .iter()
            .filter(|t| t.category == TokenCategory::Numeric)
            .collect();
        if !numerics.is_empty() {
            let mut numeric_patterns: Vec<Value> = Vec::new();
            for t in &numerics {
                numeric_patterns.push(json!({
                    "match": t.pattern,
                    "name": format!("constant.numeric.{scope}")
                }));
            }
            repository.insert(
                "numbers".to_string(),
                json!({ "patterns": numeric_patterns }),
            );
            top_patterns.push(json!({ "include": "#numbers" }));
        }

        // --- Strings ---
        let strings: Vec<&CollectedToken> = self
            .tokens
            .iter()
            .filter(|t| t.category == TokenCategory::String)
            .collect();
        if !strings.is_empty() {
            let mut string_patterns: Vec<Value> = Vec::new();
            for t in &strings {
                string_patterns.push(json!({
                    "match": t.pattern,
                    "name": format!("string.quoted.{scope}")
                }));
            }
            repository.insert(
                "strings".to_string(),
                json!({ "patterns": string_patterns }),
            );
            top_patterns.push(json!({ "include": "#strings" }));
        }

        // --- Identifiers ---
        let identifiers: Vec<&CollectedToken> = self
            .tokens
            .iter()
            .filter(|t| t.category == TokenCategory::Identifier)
            .collect();
        if !identifiers.is_empty() {
            let mut ident_patterns: Vec<Value> = Vec::new();
            for t in &identifiers {
                ident_patterns.push(json!({
                    "match": t.pattern,
                    "name": format!("variable.other.{scope}")
                }));
            }
            repository.insert(
                "identifiers".to_string(),
                json!({ "patterns": ident_patterns }),
            );
            top_patterns.push(json!({ "include": "#identifiers" }));
        }

        // --- Begin/end pairs ---
        for pair in &self.begin_end_pairs {
            top_patterns.push(json!({
                "begin": pair.begin,
                "end": pair.end,
                "name": pair.name,
                "patterns": [{ "include": "$self" }]
            }));
        }

        json!({
            "$schema": "https://raw.githubusercontent.com/martinring/tmlanguage/master/tmlanguage.json",
            "name": lang_name,
            "scopeName": format!("source.{scope}"),
            "patterns": top_patterns,
            "repository": repository
        })
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Categorise a string literal token.
fn categorise_string(value: &str) -> TokenCategory {
    if value.chars().all(|c| c.is_ascii_alphabetic() || c == '_') && !value.is_empty() {
        TokenCategory::Keyword
    } else {
        TokenCategory::Operator
    }
}

/// Categorise a regex pattern token by inspecting its content.
fn categorise_pattern(pattern: &str) -> TokenCategory {
    // Digit-heavy patterns → numeric.
    if pattern.contains("\\d") || pattern.contains("[0-9]") {
        // Check if it also has alpha chars — if so, it's likely an identifier pattern.
        if pattern.contains("a-z") || pattern.contains("A-Z") || pattern.contains("\\w") {
            return TokenCategory::Identifier;
        }
        return TokenCategory::Numeric;
    }

    // Identifier-like patterns (contain alpha ranges).
    if pattern.contains("a-z") || pattern.contains("A-Z") || pattern.contains("\\w") {
        return TokenCategory::Identifier;
    }

    // Quoted string patterns.
    if pattern.contains('"') || pattern.contains('\'') {
        return TokenCategory::String;
    }

    // Fallback: treat as identifier.
    TokenCategory::Identifier
}

/// Escape a literal string for use in a regex pattern.
fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        if "\\.*+?()[]{}|^$".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Unwrap precedence/field wrappers to reach an inner SEQ.
fn unwrap_to_seq(rule_def: &RuleDef) -> Option<&[RuleDef]> {
    match rule_def {
        RuleDef::SEQ { members } => Some(members),
        RuleDef::PREC { content, .. }
        | RuleDef::PREC_LEFT { content, .. }
        | RuleDef::PREC_RIGHT { content, .. }
        | RuleDef::PREC_DYNAMIC { content, .. }
        | RuleDef::FIELD { content, .. } => unwrap_to_seq(content),
        _ => None,
    }
}

/// Extract a string value from a RuleDef, unwrapping through FIELD wrappers.
fn extract_string_value(rule_def: &RuleDef) -> Option<String> {
    match rule_def {
        RuleDef::STRING { value } => Some(value.clone()),
        RuleDef::FIELD { content, .. } => extract_string_value(content),
        _ => None,
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rust_sitter_common::expansion::generate_grammar;
    use syn::{ItemMod, parse_quote};

    fn grammar_from_mod(m: ItemMod) -> Grammar {
        let (_, items) = m.content.unwrap();
        generate_grammar(items).unwrap().unwrap()
    }

    #[test]
    fn textmate_simple_enum() {
        let m = if let syn::Item::Mod(m) = parse_quote! {
            mod grammar {
                #[derive(rust_sitter::Rule)]
                #[language]
                pub enum Expr {
                    Number(
                        #[leaf(pattern(r"\d+"))]
                        u32
                    ),
                    Neg {
                        #[leaf("!")]
                        _bang: (),
                        value: Box<Expr>,
                    }
                }
            }
        } {
            m
        } else {
            panic!()
        };

        let grammar = grammar_from_mod(m);
        let textmate = generate_textmate(&grammar, None);
        insta::assert_snapshot!(serde_json::to_string_pretty(&textmate).unwrap());
    }

    #[test]
    fn textmate_arithmetic() {
        let m = if let syn::Item::Mod(m) = parse_quote! {
            mod grammar {
                #[derive(rust_sitter::Rule)]
                #[language]
                #[extras(re(r"\s"))]
                pub enum Expression {
                    Number(#[leaf(pattern(r"\d+"))] i32),
                    #[prec_left(1)]
                    Sub(Box<Expression>, #[leaf("-")] (), Box<Expression>),
                    #[prec_left(2)]
                    Mul(Box<Expression>, #[leaf("*")] (), Box<Expression>),
                    Let(LetExpression),
                    Print(PrintExpression),
                }

                #[derive(rust_sitter::Rule)]
                pub struct LetExpression {
                    #[text("let")]
                    _let: (),
                    pub var: Ident,
                    #[text("=")]
                    _eq: (),
                    pub val: Box<Expression>,
                }

                #[derive(rust_sitter::Rule)]
                pub struct PrintExpression {
                    #[text("print")]
                    _print: (),
                    #[text("(")]
                    _lparen: (),
                    #[sep_by(",")]
                    inputs: Vec<Expression>,
                    #[text(")")]
                    _rparen: (),
                }

                #[derive(rust_sitter::Rule)]
                pub struct Ident(#[leaf(re(r"[a-zA-Z_][a-zA-Z_0-9]*"))] String);
            }
        } {
            m
        } else {
            panic!()
        };

        let grammar = grammar_from_mod(m);
        let textmate = generate_textmate(&grammar, None);
        insta::assert_snapshot!(serde_json::to_string_pretty(&textmate).unwrap());
    }

    #[test]
    fn textmate_if_statement() {
        let m = if let syn::Item::Mod(m) = parse_quote! {
            mod grammar {
                #[derive(rust_sitter::Rule)]
                #[language]
                #[word(Identifier)]
                pub struct Program(pub Vec<Statement>);

                #[derive(rust_sitter::Rule)]
                pub enum Statement {
                    ExpressionStatement(ExpressionStatement),
                    IfStatement(Box<IfStatement>),
                }

                #[derive(rust_sitter::Rule)]
                pub enum Expression {
                    Identifier(Identifier),
                    Number(Number),
                }

                #[derive(rust_sitter::Rule)]
                pub struct ExpressionStatement {
                    pub expression: Expression,
                    #[leaf(";")]
                    pub _semicolon: (),
                }

                #[derive(rust_sitter::Rule)]
                #[prec_dynamic(1)]
                pub struct IfStatement {
                    #[leaf("if")]
                    pub _if: (),
                    #[leaf("(")]
                    pub _lparen: (),
                    pub expression: Expression,
                    #[leaf(")")]
                    pub _rparen: (),
                    #[leaf("{")]
                    pub _lbrace: (),
                    pub statement: Statement,
                    #[leaf("}")]
                    pub _rbrace: (),
                    pub else_clause: Option<IfStatementElse>,
                }

                #[derive(rust_sitter::Rule)]
                pub struct IfStatementElse {
                    #[leaf("else")]
                    pub _else: (),
                    #[leaf("{")]
                    pub _lbrace: (),
                    pub statement: Statement,
                    #[leaf("}")]
                    pub _rbrace: (),
                }

                #[derive(rust_sitter::Rule)]
                #[leaf(pattern("[a-zA-Z_][a-zA-Z0-9_]*"))]
                pub struct Identifier;

                #[derive(rust_sitter::Rule)]
                pub struct Number(#[leaf(pattern(r"\d+"))] ());
            }
        } {
            m
        } else {
            panic!()
        };

        let grammar = grammar_from_mod(m);
        let textmate = generate_textmate(&grammar, None);
        insta::assert_snapshot!(serde_json::to_string_pretty(&textmate).unwrap());
    }

    #[test]
    fn textmate_with_extras() {
        let m = if let syn::Item::Mod(m) = parse_quote! {
            mod grammar {
                #[derive(rust_sitter::Rule)]
                #[language]
                #[extras(re(r"\s"), re(r"//[^\n]*"))]
                pub enum Expression {
                    Number(
                        #[leaf(re(r"\d+"))]
                        i32
                    ),
                }
            }
        } {
            m
        } else {
            panic!()
        };

        let grammar = grammar_from_mod(m);
        let textmate = generate_textmate(&grammar, None);
        let json_str = serde_json::to_string_pretty(&textmate).unwrap();
        // Verify the comment pattern is present in the output
        assert!(
            json_str.contains("comment.line.double-slash"),
            "TextMate output should contain line comment scope: {json_str}"
        );
        insta::assert_snapshot!(json_str);
    }

    #[test]
    fn textmate_with_token_wrapped_extras() {
        let m = if let syn::Item::Mod(m) = parse_quote! {
            mod grammar {
                #[derive(rust_sitter::Rule)]
                #[language]
                #[extras(
                    token(re(r"\s+")),
                    token(re(r"//[^\n]*")),
                    token(re(r"/\*[^*]*\*+(?:[^/*][^*]*\*+)*/"))
                )]
                pub enum Expression {
                    Number(
                        #[leaf(re(r"\d+"))]
                        i32
                    ),
                }
            }
        } {
            m
        } else {
            panic!()
        };

        let grammar = grammar_from_mod(m);
        let textmate = generate_textmate(&grammar, None);
        let json_str = serde_json::to_string_pretty(&textmate).unwrap();
        // Verify both comment patterns are present
        assert!(
            json_str.contains("comment.line.double-slash"),
            "TextMate output should contain line comment scope: {json_str}"
        );
        assert!(
            json_str.contains("comment.block"),
            "TextMate output should contain block comment scope: {json_str}"
        );
        insta::assert_snapshot!(json_str);
    }
}
