// SPDX-License-Identifier: MIT

// TODO: Switch on which version we are using specifically.
const GENERATED_SEMANTIC_VERSION: Option<(u8, u8, u8)> = Some((0, 26, 0));

use std::io::Write;
use std::path::{Path, PathBuf};

use tree_sitter_generate::generate_parser_for_grammar;

pub mod textmate;

/// Using the `cc` crate, generates and compiles a C parser with Tree Sitter
/// for every Krust Sitter grammar found in the given module and recursive
/// submodules.
pub fn build_parser<P>(root_file: &P)
where
    P: AsRef<Path> + ?Sized,
{
    ParserBuilder::default().build(root_file);
}

/// Builder for generating TextMate grammars from Krust Sitter grammar definitions.
///
/// Similar to `ParserBuilder`, but produces a `.tmLanguage.json` value instead of
/// a C parser.
#[derive(Default)]
pub struct TextMateBuilder {
    pub scope_name: Option<String>,
}

impl TextMateBuilder {
    /// Set a custom scope name (e.g. `"source.karu"`). If not set, the grammar name is used.
    pub fn scope_name(mut self, scope: impl Into<String>) -> Self {
        self.scope_name = Some(scope.into());
        self
    }

    /// Parse the grammar from the given Rust source file and generate a TextMate grammar.
    ///
    /// Returns `Some(json)` if a grammar was found, `None` otherwise.
    pub fn build<P>(self, root_file: &P) -> Option<serde_json::Value>
    where
        P: AsRef<Path> + ?Sized,
    {
        let root_file = syn_inline_mod::parse_and_inline_modules(root_file.as_ref());
        match krust_sitter_common::expansion::generate_grammar(root_file.items) {
            Err(e) => panic!("{e}"),
            Ok(None) => None,
            Ok(Some(grammar)) => {
                let result = textmate::generate_textmate(&grammar, self.scope_name.as_deref());
                Some(result)
            }
        }
    }

    /// Parse the grammar and generate preview highlighter files (JS + CSS) for
    /// markdown preview integration.
    pub fn build_preview<P>(self, root_file: &P, lang_name: &str) -> Option<textmate::PreviewFiles>
    where
        P: AsRef<Path> + ?Sized,
    {
        let root_file = syn_inline_mod::parse_and_inline_modules(root_file.as_ref());
        match krust_sitter_common::expansion::generate_grammar(root_file.items) {
            Err(e) => panic!("{e}"),
            Ok(None) => None,
            Ok(Some(grammar)) => {
                let result =
                    textmate::generate_preview(&grammar, self.scope_name.as_deref(), lang_name);
                Some(result)
            }
        }
    }
}

#[derive(Default)]
pub struct ParserBuilder {
    pub output: Option<PathBuf>,
}

impl ParserBuilder {
    pub fn output(mut self, output: impl Into<PathBuf>) -> Self {
        self.output = Some(output.into());
        self
    }

    pub fn build<P>(self, root_file: &P)
    where
        P: AsRef<Path> + ?Sized,
    {
        let root_file = syn_inline_mod::parse_and_inline_modules(root_file.as_ref());
        match krust_sitter_common::expansion::generate_grammar(root_file.items) {
            Err(e) => panic!("{e}"),
            Ok(None) => {}
            Ok(Some(grammar)) => {
                let grammar = serde_json::to_value(grammar).unwrap();
                // TODO: We want to generate better errors here as well. However, it isn't really
                // possible to generate it until we can produce a full grammar, which we also can't do
                // if we derive on Rule.
                if let Err(e) = generate_parser(&grammar, self.output.as_deref()) {
                    panic!("{e}");
                }
            }
        }
    }
}

// TODO: Rewrite this function to support specifying the out dir and target manually, to allow
// generating the parser to a local folder for easier integration with external text editors.
fn generate_parser(grammar: &serde_json::Value, out_dir: Option<&Path>) -> Result<(), String> {
    let grammar_string = grammar.to_string();
    let (grammar_name, grammar_c) =
        match generate_parser_for_grammar(&grammar_string, GENERATED_SEMANTIC_VERSION) {
            Ok(o) => o,
            Err(e) => {
                // Doing it this way produces a clean error from tree-sitter on failure.
                return Err(format!("generation error: {e}"));
            }
        };
    let tempfile = tempfile::Builder::new()
        .prefix("grammar")
        .tempdir()
        .unwrap();

    let dir = if let Some(out) = out_dir {
        out
    } else {
        tempfile.path()
    };
    let sysroot_dir = write_grammar_and_c_to_dir(&grammar_name, grammar, &grammar_c, dir);
    // Check if we have an additional output directory.
    if let Ok(output) = std::env::var("RUST_SITTER_PARSER_OUTPUT") {
        let output: &Path = output.as_ref();
        write_grammar_and_c_to_dir(&grammar_name, grammar, &grammar_c, output);
    }

    let mut c_config = cc::Build::new();
    c_config.std("c11").include(dir);
    if let Some(sysroot_dir) = &sysroot_dir {
        c_config.include(sysroot_dir);
    }
    c_config
        .flag_if_supported("-Wno-unused-label")
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-unused-but-set-variable")
        .flag_if_supported("-Wno-trigraphs")
        .flag_if_supported("-Wno-everything");
    c_config.file(dir.join("parser.c"));

    c_config.compile(&grammar_name);
    Ok(())
}

fn write_grammar_and_c_to_dir(
    grammar_name: &str,
    grammar: &serde_json::Value,
    grammar_c: &str,
    dir: &Path,
) -> Option<PathBuf> {
    let grammar_file = dir.join("parser.c");
    let mut f = std::fs::File::create(grammar_file).unwrap();

    f.write_all(grammar_c.as_bytes()).unwrap();
    drop(f);

    // emit grammar into the build out_dir
    let mut grammar_json_file =
        std::fs::File::create(dir.join(format!("{grammar_name}.json"))).unwrap();
    grammar_json_file
        .write_all(serde_json::to_string_pretty(grammar).unwrap().as_bytes())
        .unwrap();
    drop(grammar_json_file);

    let header_dir = dir.join("tree_sitter");
    std::fs::create_dir_all(&header_dir).unwrap();
    let mut parser_file = std::fs::File::create(header_dir.join("parser.h")).unwrap();
    parser_file
        .write_all(tree_sitter::PARSER_HEADER.as_bytes())
        .unwrap();
    drop(parser_file);

    if std::env::var("TARGET").is_ok_and(|target| target.starts_with("wasm32")) {
        let sysroot_dir = dir.join("sysroot");
        std::fs::create_dir_all(&sysroot_dir).unwrap();

        let headers = [
            (
                "stdint.h",
                include_bytes!("wasm-sysroot/stdint.h").as_slice(),
            ),
            (
                "stdlib.h",
                include_bytes!("wasm-sysroot/stdlib.h").as_slice(),
            ),
            ("stdio.h", include_bytes!("wasm-sysroot/stdio.h").as_slice()),
            (
                "stdbool.h",
                include_bytes!("wasm-sysroot/stdbool.h").as_slice(),
            ),
        ];

        for (name, contents) in headers {
            let mut file = std::fs::File::create(sysroot_dir.join(name)).unwrap();
            file.write_all(contents).unwrap();
        }

        Some(sysroot_dir)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use syn::{ItemMod, parse_quote};

    use super::GENERATED_SEMANTIC_VERSION;
    // use krust_sitter_common::expansion::generate_grammar;
    use tree_sitter_generate::generate_parser_for_grammar;
    fn generate_grammar(item: ItemMod) -> serde_json::Value {
        let (_, items) = item.content.unwrap();
        serde_json::to_value(
            krust_sitter_common::expansion::generate_grammar(items)
                .unwrap()
                .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn enum_with_named_field() {
        let m = if let syn::Item::Mod(m) = parse_quote! {
            mod grammar {
                #[derive(krust_sitter::Rule)]
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

        let grammar = generate_grammar(m);
        insta::assert_snapshot!(grammar);
        generate_parser_for_grammar(&grammar.to_string(), GENERATED_SEMANTIC_VERSION).unwrap();
    }

    #[test]
    fn enum_transformed_fields() {
        let m = if let syn::Item::Mod(m) = parse_quote! {
            mod grammar {
                #[derive(krust_sitter::Rule)]
                #[language]
                pub enum Expression {
                    Number(
                        #[leaf(pattern(r"\d+"))]
                        #[transform(|v: &str| v.parse::<i32>().unwrap())]
                        i32
                    ),
                }
            }
        } {
            m
        } else {
            panic!()
        };

        let grammar = generate_grammar(m);
        insta::assert_snapshot!(grammar);
        generate_parser_for_grammar(&grammar.to_string(), GENERATED_SEMANTIC_VERSION).unwrap();
    }

    #[test]
    fn enum_recursive() {
        let m = if let syn::Item::Mod(m) = parse_quote! {
            mod grammar {
                #[derive(krust_sitter::Rule)]
                #[language]
                pub enum Expression {
                    Number(
                        #[leaf(pattern(r"\d+"))]
                        i32
                    ),
                    Neg(
                        #[leaf("-")]
                        (),
                        Box<Expression>
                    ),
                }
            }
        } {
            m
        } else {
            panic!()
        };

        let grammar = generate_grammar(m);
        insta::assert_snapshot!(grammar);
        generate_parser_for_grammar(&grammar.to_string(), GENERATED_SEMANTIC_VERSION).unwrap();
    }

    #[test]
    fn enum_prec_left() {
        let m = if let syn::Item::Mod(m) = parse_quote! {
            mod grammar {
                #[derive(krust_sitter::Rule)]
                #[language]
                pub enum Expression {
                    Number(
                        #[leaf(pattern(r"\d+"))]
                        i32
                    ),
                    #[prec_left(1)]
                    Sub(
                        Box<Expression>,
                        #[leaf("-")]
                        (),
                        Box<Expression>
                    ),
                }
            }
        } {
            m
        } else {
            panic!()
        };

        let grammar = generate_grammar(m);
        insta::assert_snapshot!(grammar);
        generate_parser_for_grammar(&grammar.to_string(), GENERATED_SEMANTIC_VERSION).unwrap();
    }

    #[test]
    fn enum_conflicts_prec_dynamic() {
        let m = if let syn::Item::Mod(m) = parse_quote! {
            mod grammar {
                #[derive(krust_sitter::Rule)]
                #[language]
                #[word(Identifier)]
                pub struct Program(pub Vec<Statement>);

                #[derive(krust_sitter::Rule)]
                pub enum Statement {
                    ExpressionStatement(ExpressionStatement),
                    IfStatement(Box<IfStatement>),
                }

                #[derive(krust_sitter::Rule)]
                pub enum Expression {
                    Identifier(Identifier),
                    Number(Number),
                    BinaryExpression(Box<BinaryExpression>),
                }

                #[derive(krust_sitter::Rule)]
                #[prec_left(1)]
                pub struct BinaryExpression {
                    pub expression: Expression,
                    pub binary_expression_inner: BinaryExpressionInner,
                    pub expression2: Expression,
                }

                #[derive(krust_sitter::Rule)]
                pub enum BinaryExpressionInner {
                    String(#[leaf("+")] ()),
                    String2(#[leaf("-")] ()),
                    String3(#[leaf("*")] ()),
                    String4(#[leaf("/")] ()),
                }

                #[derive(krust_sitter::Rule)]
                pub struct ExpressionStatement {
                    pub expression: Expression,
                    #[leaf(";")]
                    pub _semicolon: (),
                }

                #[derive(krust_sitter::Rule)]
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
                    pub if_statement_inner: Option<IfStatementElse>,
                }

                #[derive(krust_sitter::Rule)]
                pub struct IfStatementElse {
                    #[leaf("else")]
                    pub _else: (),
                    #[leaf("{")]
                    pub _lbrace: (),
                    pub statement: Statement,
                    #[leaf("}")]
                    pub _rbrace: (),
                }

                #[derive(krust_sitter::Rule)]
                #[leaf(pattern("[a-zA-Z_][a-zA-Z0-9_]*"))]
                pub struct Identifier;

                #[derive(krust_sitter::Rule)]
                pub struct Number(#[leaf(pattern("\\d+"))] ());
            }
        } {
            m
        } else {
            panic!()
        };

        let grammar = generate_grammar(m);
        insta::assert_snapshot!(grammar);
        generate_parser_for_grammar(&grammar.to_string(), GENERATED_SEMANTIC_VERSION).unwrap();
    }

    #[test]
    fn grammar_with_extras() {
        let m = if let syn::Item::Mod(m) = parse_quote! {
            mod grammar {
                #[derive(krust_sitter::Rule)]
                #[language]
                #[extras(
                    re(r"\s")
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

        let grammar = generate_grammar(m);
        insta::assert_snapshot!(grammar);
        generate_parser_for_grammar(&grammar.to_string(), GENERATED_SEMANTIC_VERSION).unwrap();
    }

    #[test]
    fn grammar_unboxed_field() {
        let m = if let syn::Item::Mod(m) = parse_quote! {
            mod grammar {
                #[derive(krust_sitter::Rule)]
                #[language]
                pub struct Language {
                    e: Expression,
                }

                #[derive(krust_sitter::Rule)]
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

        let grammar = generate_grammar(m);
        insta::assert_snapshot!(grammar);
        generate_parser_for_grammar(&grammar.to_string(), GENERATED_SEMANTIC_VERSION).unwrap();
    }

    #[test]
    fn grammar_repeat() {
        let m = if let syn::Item::Mod(m) = parse_quote! {
            pub mod grammar {
                #[derive(krust_sitter::Rule)]
                #[language]
                #[extras(
                    re(r"\s")
                )]
                pub struct NumberList {
                    #[sep_by(",")]
                    numbers: Vec<Number>,
                }

                #[derive(Rule)]
                pub struct Number {
                    #[leaf(re(r"\d+"))]
                    v: i32,
                }
            }
        } {
            m
        } else {
            panic!()
        };

        let grammar = generate_grammar(m);
        insta::assert_snapshot!(grammar);
        generate_parser_for_grammar(&grammar.to_string(), GENERATED_SEMANTIC_VERSION).unwrap();
    }

    #[test]
    fn grammar_repeat_no_delimiter() {
        let m = if let syn::Item::Mod(m) = parse_quote! {
            pub mod grammar {
                #[derive(krust_sitter::Rule)]
                #[language]
                #[extras(
                    re(r"\s")
                )]
                pub struct NumberList {
                    numbers: Vec<Number>,
                }

                #[derive(krust_sitter::Rule)]
                pub struct Number {
                    #[leaf(re(r"\d+"))]
                    v: i32,
                }
            }
        } {
            m
        } else {
            panic!()
        };

        let grammar = generate_grammar(m);
        insta::assert_snapshot!(grammar);
        generate_parser_for_grammar(&grammar.to_string(), GENERATED_SEMANTIC_VERSION).unwrap();
    }

    #[test]
    fn grammar_repeat1() {
        let m = if let syn::Item::Mod(m) = parse_quote! {
            pub mod grammar {
                #[derive(krust_sitter::Rule)]
                #[language]
                #[extras(
                    re(r"\s")
                )]
                pub struct NumberList {
                    #[repeat(non_empty = true)]
                    #[delimited(",")]
                    numbers: Vec<Number>,
                }

                #[derive(krust_sitter::Rule)]
                pub struct Number {
                    #[leaf(re(r"\d+"))]
                    v: i32,
                }
            }
        } {
            m
        } else {
            panic!()
        };

        let grammar = generate_grammar(m);
        insta::assert_snapshot!(grammar);
        generate_parser_for_grammar(&grammar.to_string(), GENERATED_SEMANTIC_VERSION).unwrap();
    }

    #[test]
    fn struct_optional() {
        let m = if let syn::Item::Mod(m) = parse_quote! {
            mod grammar {
                #[derive(krust_sitter::Rule)]
                #[language]
                pub struct Language {
                    #[leaf(re(r"\d+"))]
                    v: Option<i32>,
                    #[leaf(re(r" "))]
                    space: (),
                    t: Option<Number>,
                }

                #[derive(krust_sitter::Rule)]
                pub struct Number {
                    #[leaf(re(r"\d+"))]
                    v: i32
                }
            }
        } {
            m
        } else {
            panic!()
        };

        let grammar = generate_grammar(m);
        insta::assert_snapshot!(grammar);
        generate_parser_for_grammar(&grammar.to_string(), GENERATED_SEMANTIC_VERSION).unwrap();
    }

    #[test]
    fn enum_with_unamed_vector() {
        let m = if let syn::Item::Mod(m) = parse_quote! {
            mod grammar {
                #[derive(krust_sitter::Rule)]
                pub struct Number {
                        #[leaf(re(r"\d+"))]
                        value: u32
                }

                #[derive(krust_sitter::Rule)]
                #[language]
                pub enum Expr {
                    Numbers(
                        #[repeat1]
                        Vec<Number>
                    )
                }
            }
        } {
            m
        } else {
            panic!()
        };

        let grammar = generate_grammar(m);
        insta::assert_snapshot!(grammar);
        generate_parser_for_grammar(&grammar.to_string(), GENERATED_SEMANTIC_VERSION).unwrap();
    }

    #[test]
    fn spanned_in_vec() {
        let m = if let syn::Item::Mod(m) = parse_quote! {
            mod grammar {
                use krust_sitter::Spanned;

                #[derive(krust_sitter::Rule)]
                #[language]
                #[extras(
                    re(r"\s")
                )]
                pub struct NumberList {
                    #[leaf(re(r"\d+"))]
                    numbers: Vec<Spanned<i32>>,
                }
            }
        } {
            m
        } else {
            panic!()
        };

        let grammar = generate_grammar(m);
        insta::assert_snapshot!(grammar);
        generate_parser_for_grammar(&grammar.to_string(), GENERATED_SEMANTIC_VERSION).unwrap();
    }

    #[test]
    fn immediate() {
        let m = if let syn::Item::Mod(m) = parse_quote! {
            mod grammar {
                #[derive(krust_sitter::Rule)]
                #[language]
                #[extras(
                    re(r"\s")
                )]
                pub struct StringFragment(
                    #[immediate]
                    #[prec(1)]
                    #[leaf(pattern(r#"[^"\\]+"#))]
                    ()
                );
            }
        } {
            m
        } else {
            panic!()
        };

        let grammar = generate_grammar(m);
        insta::assert_snapshot!(grammar);
        generate_parser_for_grammar(&grammar.to_string(), GENERATED_SEMANTIC_VERSION).unwrap();
    }
}
