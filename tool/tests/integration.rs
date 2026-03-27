/// Integration tests for grammar generation.
///
/// These tests verify that Rust Sitter grammar definitions are correctly
/// translated into tree-sitter grammar JSON and that `generate_parser_for_grammar`
/// accepts that JSON without error.
///
/// Note: these tests intentionally do **not** call `build_parser` / `cc::Build::compile`.
/// Spawning a C compiler for every test in parallel causes file-system conflicts
/// on the shared output path and makes the test suite hang.  Grammar generation
/// (the Rust → JSON step) is fast, pure, and sufficient to validate correctness
/// here; the compilation step is already exercised by the `example` crate's
/// build script.
use rust_sitter_common::expansion::generate_grammar as expand_grammar;
use syn::{Item, ItemMod, parse_quote};
use tree_sitter_generate::generate_parser_for_grammar;

/// Tree-sitter ABI version targeted by this workspace.
const GENERATED_SEMANTIC_VERSION: Option<(u8, u8, u8)> = Some((0, 26, 0));

/// Convenience wrapper: turn an `ItemMod` into a validated grammar JSON value
/// and confirm that `generate_parser_for_grammar` accepts it.
fn check_grammar(m: ItemMod) {
    let (_, items) = m.content.unwrap();
    let grammar = serde_json::to_value(expand_grammar(items).unwrap().unwrap()).unwrap();
    generate_parser_for_grammar(&grammar.to_string(), GENERATED_SEMANTIC_VERSION).unwrap();
}

#[test]
fn enum_with_named_field() {
    let m: Item = parse_quote! {
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
    };
    check_grammar(match m {
        Item::Mod(m) => m,
        _ => panic!("expected mod"),
    });
}

#[test]
fn enum_transformed_fields() {
    let m: Item = parse_quote! {
        mod grammar {
            #[derive(rust_sitter::Rule)]
            #[language]
            pub enum Expression {
                Number(
                    #[leaf(pattern(r"\d+"))]
                    #[transform(|v: &str| v.parse::<i32>().unwrap())]
                    i32
                ),
            }
        }
    };
    check_grammar(match m {
        Item::Mod(m) => m,
        _ => panic!("expected mod"),
    });
}

#[test]
fn enum_recursive() {
    let m: Item = parse_quote! {
        mod grammar {
            #[derive(rust_sitter::Rule)]
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
    };
    check_grammar(match m {
        Item::Mod(m) => m,
        _ => panic!("expected mod"),
    });
}

#[test]
fn enum_prec_left() {
    let m: Item = parse_quote! {
        mod grammar {
            #[derive(rust_sitter::Rule)]
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
    };
    check_grammar(match m {
        Item::Mod(m) => m,
        _ => panic!("expected mod"),
    });
}

#[test]
fn enum_conflicts_prec_dynamic() {
    let m: Item = parse_quote! {
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
                BinaryExpression(Box<BinaryExpression>),
            }

            #[derive(rust_sitter::Rule)]
            #[prec_left(1)]
            pub struct BinaryExpression {
                pub expression: Expression,
                pub binary_expression_inner: BinaryExpressionInner,
                pub expression2: Expression,
            }

            #[derive(rust_sitter::Rule)]
            pub enum BinaryExpressionInner {
                String(#[leaf("+")] ()),
                String2(#[leaf("-")] ()),
                String3(#[leaf("*")] ()),
                String4(#[leaf("/")] ()),
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
                pub if_statement_inner: Option<IfStatementElse>,
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
            pub struct Number(#[leaf(pattern("\\d+"))] ());
        }
    };
    check_grammar(match m {
        Item::Mod(m) => m,
        _ => panic!("expected mod"),
    });
}

#[test]
fn grammar_with_extras() {
    let m: Item = parse_quote! {
        mod grammar {
            #[derive(rust_sitter::Rule)]
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
    };
    check_grammar(match m {
        Item::Mod(m) => m,
        _ => panic!("expected mod"),
    });
}

#[test]
fn grammar_unboxed_field() {
    let m: Item = parse_quote! {
        mod grammar {
            #[derive(rust_sitter::Rule)]
            #[language]
            pub struct Language {
                e: Expression,
            }

            #[derive(rust_sitter::Rule)]
            pub enum Expression {
                Number(
                    #[leaf(re(r"\d+"))]
                    i32
                ),
            }
        }
    };
    check_grammar(match m {
        Item::Mod(m) => m,
        _ => panic!("expected mod"),
    });
}

#[test]
fn grammar_repeat() {
    let m: Item = parse_quote! {
        pub mod grammar {
            #[derive(rust_sitter::Rule)]
            #[language]
            #[extras(
                re(r"\s")
            )]
            pub struct NumberList {
                #[sep_by(",")]
                numbers: Vec<Number>,
            }

            #[derive(rust_sitter::Rule)]
            pub struct Number {
                #[leaf(re(r"\d+"))]
                v: i32,
            }
        }
    };
    check_grammar(match m {
        Item::Mod(m) => m,
        _ => panic!("expected mod"),
    });
}

#[test]
fn grammar_repeat_no_delimiter() {
    let m: Item = parse_quote! {
        pub mod grammar {
            #[derive(rust_sitter::Rule)]
            #[language]
            #[extras(
                re(r"\s")
            )]
            pub struct NumberList {
                numbers: Vec<Number>,
            }

            #[derive(rust_sitter::Rule)]
            pub struct Number {
                #[leaf(re(r"\d+"))]
                v: i32,
            }
        }
    };
    check_grammar(match m {
        Item::Mod(m) => m,
        _ => panic!("expected mod"),
    });
}

#[test]
fn grammar_repeat1() {
    let m: Item = parse_quote! {
        pub mod grammar {
            #[derive(rust_sitter::Rule)]
            #[language]
            #[extras(
                re(r"\s")
            )]
            pub struct NumberList {
                #[repeat(non_empty = true)]
                #[delimited(",")]
                numbers: Vec<Number>,
            }

            #[derive(rust_sitter::Rule)]
            pub struct Number {
                #[leaf(re(r"\d+"))]
                v: i32,
            }
        }
    };
    check_grammar(match m {
        Item::Mod(m) => m,
        _ => panic!("expected mod"),
    });
}

#[test]
fn struct_optional() {
    let m: Item = parse_quote! {
        mod grammar {
            #[derive(rust_sitter::Rule)]
            #[language]
            pub struct Language {
                #[leaf(re(r"\d+"))]
                v: Option<i32>,
                #[leaf(re(r" "))]
                space: (),
                t: Option<Number>,
            }

            #[derive(rust_sitter::Rule)]
            pub struct Number {
                #[leaf(re(r"\d+"))]
                v: i32
            }
        }
    };
    check_grammar(match m {
        Item::Mod(m) => m,
        _ => panic!("expected mod"),
    });
}

#[test]
fn enum_with_unamed_vector() {
    let m: Item = parse_quote! {
        mod grammar {
            #[derive(rust_sitter::Rule)]
            pub struct Number {
                #[leaf(re(r"\d+"))]
                value: u32
            }

            #[derive(rust_sitter::Rule)]
            #[language]
            pub enum Expr {
                Numbers(
                    #[repeat1]
                    Vec<Number>
                )
            }
        }
    };
    check_grammar(match m {
        Item::Mod(m) => m,
        _ => panic!("expected mod"),
    });
}
