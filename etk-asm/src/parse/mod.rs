mod args;
pub(crate) mod error;
mod parser {
    #![allow(clippy::upper_case_acronyms)]

    use pest_derive::Parser;

    #[derive(Parser)]
    #[grammar = "parse/asm.pest"]
    pub(super) struct AsmParser;
}

use crate::ast::Node;
use crate::ops::{AbstractOp, Imm, Op, Specifier};

use pest::Parser;

use self::args::{Label, Signature};
use self::error::ParseError;
use self::parser::{AsmParser, Rule};

use sha3::{Digest, Keccak256};

use snafu::OptionExt;

use std::path::PathBuf;

pub(crate) fn parse_asm(asm: &str) -> Result<Vec<Node>, ParseError> {
    let mut program: Vec<Node> = Vec::new();

    let pairs = AsmParser::parse(Rule::program, asm)?;
    for pair in pairs {
        match pair.as_rule() {
            Rule::inst_macro => {
                let mut pairs = pair.into_inner();
                let inst_macro = pairs.next().unwrap();
                assert!(pairs.next().is_none());
                let node = parse_inst_macro(inst_macro)?;
                program.push(node);
            }
            Rule::label_defn => {
                let mut pair = pair.into_inner();
                let label = pair.next().unwrap();
                let txt = label.as_str();
                program.push(AbstractOp::Label(txt.into()).into());
            }
            Rule::push => {
                program.push(parse_push(pair)?.into());
            }
            Rule::op => {
                let spec: Specifier = pair.as_str().parse().unwrap();
                let op = Op::new(spec).unwrap();
                let aop = AbstractOp::Op(op);
                program.push(aop.into());
            }
            _ => continue,
        }
    }

    Ok(program)
}

fn parse_push(pair: pest::iterators::Pair<Rule>) -> Result<AbstractOp, ParseError> {
    let mut pair = pair.into_inner();
    let size = pair.next().unwrap();
    let size: usize = size.as_str().parse().unwrap();
    let operand = pair.next().unwrap();

    let spec = Specifier::push(size as u32).unwrap();

    let op = match operand.as_rule() {
        Rule::binary => {
            let raw = operand.as_str();
            let imm = radix_str_to_vec(&raw[2..], 2, size)?;
            AbstractOp::with_immediate(spec, imm.as_ref())
                .ok()
                .context(error::ImmediateTooLarge)?
        }
        Rule::octal => {
            let raw = operand.as_str();
            let imm = radix_str_to_vec(&raw[2..], 8, size)?;
            AbstractOp::with_immediate(spec, imm.as_ref())
                .ok()
                .context(error::ImmediateTooLarge)?
        }
        Rule::decimal => {
            let raw = operand.as_str();
            let imm = radix_str_to_vec(raw, 10, size)?;
            AbstractOp::with_immediate(spec, imm.as_ref())
                .ok()
                .context(error::ImmediateTooLarge)?
        }
        Rule::hex => {
            let raw = operand.as_str();
            let imm = hex::decode(&raw[2..]).unwrap();
            AbstractOp::with_immediate(spec, imm.as_ref())
                .ok()
                .context(error::ImmediateTooLarge)?
        }
        Rule::selector => {
            let raw = operand.into_inner().next().unwrap().as_str();
            let mut hasher = Keccak256::new();
            hasher.update(raw.as_bytes());
            AbstractOp::with_immediate(spec, &hasher.finalize()[0..(spec.size() - 1) as usize])
                .ok()
                .context(error::ImmediateTooLarge)?
        }
        Rule::label => {
            let label = operand.as_str().to_string();
            AbstractOp::with_label(spec, label)
        }
        r => unreachable!(format!("{:?}", r)),
    };

    Ok(op)
}

fn parse_inst_macro(pair: pest::iterators::Pair<Rule>) -> Result<Node, ParseError> {
    let rule = pair.as_rule();

    let node = match rule {
        Rule::import => {
            let args = <(PathBuf,)>::parse_arguments(pair.into_inner())?;
            Node::Import(args.0)
        }

        Rule::include => {
            let args = <(PathBuf,)>::parse_arguments(pair.into_inner())?;
            Node::Include(args.0)
        }

        Rule::include_hex => {
            let args = <(PathBuf,)>::parse_arguments(pair.into_inner())?;
            Node::IncludeHex(args.0)
        }

        Rule::push_macro => {
            // TODO: This should accept labels or literals, not just labels.
            let args = <(Label,)>::parse_arguments(pair.into_inner())?;
            let arg = Imm::from(args.0 .0);
            Node::Op(AbstractOp::Push(arg))
        }

        _ => unreachable!(),
    };
    Ok(node)
}

fn radix_str_to_vec(s: &str, radix: u32, min: usize) -> Result<Vec<u8>, ParseError> {
    let n = u128::from_str_radix(s, radix)
        .ok()
        .context(error::ImmediateTooLarge)?;

    let msb = 128 - n.leading_zeros();
    let mut len = (msb / 8) as usize;
    if msb % 8 != 0 {
        len += 1;
    }

    len = std::cmp::max(len, min);

    Ok(n.to_be_bytes()[16 - len..].to_vec())
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;

    use crate::ops::Imm;

    use hex_literal::hex;

    use super::*;

    macro_rules! nodes {
        ($($x:expr),+ $(,)?) => (
            vec![$(Node::from($x)),+]
        );
    }

    #[test]
    fn parse_ops() {
        let asm = r#"
            stop
            pc
            gas
            xor
        "#;
        let expected = nodes![Op::Stop, Op::GetPc, Op::Gas, Op::Xor];
        assert_matches!(parse_asm(asm), Ok(e) if e == expected);
    }

    #[test]
    fn parse_single_line() {
        let asm = r#"
            push1 0b0; push1 0b1
        "#;
        let expected = nodes![Op::Push1(Imm::from([0])), Op::Push1(Imm::from([1]))];
        assert_matches!(parse_asm(asm), Ok(e) if e == expected);
    }

    #[test]
    fn parse_mixed_lines() {
        let asm = r#"
            push1 0b0; push1 0b1
            push1 0b1
        "#;
        let expected = nodes![
            Op::Push1(Imm::from([0])),
            Op::Push1(Imm::from([1])),
            Op::Push1(Imm::from([1]))
        ];
        assert_matches!(parse_asm(asm), Ok(e) if e == expected);
    }

    #[test]
    fn parse_push_binary() {
        let asm = r#"
            # simple cases
            push1 0b0
            push1 0b1
        "#;
        let expected = nodes![Op::Push1(Imm::from([0])), Op::Push1(Imm::from([1]))];
        assert_matches!(parse_asm(asm), Ok(e) if e == expected);
    }

    #[test]
    fn parse_push_octal() {
        let asm = r#"
            # simple cases
            push1 0o0
            push1 0o7
            push2 0o400
        "#;
        let expected = nodes![
            Op::Push1(Imm::from([0])),
            Op::Push1(Imm::from([7])),
            Op::Push2(Imm::from([1, 0])),
        ];
        assert_matches!(parse_asm(asm), Ok(e) if e == expected);
    }

    #[test]
    fn parse_push_decimal() {
        let asm = r#"
            # simple cases
            push1 0
            push1 1

            # left-pad values too small
            push2 42

            # barely enough for 2 bytes
            push2 256

            # just enough for 4 bytes
            push4 4294967295
        "#;
        let expected = nodes![
            Op::Push1(Imm::from([0])),
            Op::Push1(Imm::from([1])),
            Op::Push2(Imm::from([0, 42])),
            Op::Push2(Imm::from(hex!("0100"))),
            Op::Push4(Imm::from(hex!("ffffffff"))),
        ];
        assert_matches!(parse_asm(asm), Ok(e) if e == expected);

        let asm = "push1 256";
        assert_matches!(parse_asm(asm), Err(ParseError::ImmediateTooLarge { .. }));
    }

    #[test]
    fn parse_push_hex() {
        let asm = r#"
            push1 0x01 # comment
            push1 0x42
            push2 0x0102
            push4 0x01020304
            push8 0x0102030405060708
            push16 0x0102030405060708090a0b0c0d0e0f10
            push24 0x0102030405060708090a0b0c0d0e0f101112131415161718
            push32 0x0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20
        "#;
        let expected = nodes![
            Op::Push1(Imm::from(hex!("01"))),
            Op::Push1(Imm::from(hex!("42"))),
            Op::Push2(Imm::from(hex!("0102"))),
            Op::Push4(Imm::from(hex!("01020304"))),
            Op::Push8(Imm::from(hex!("0102030405060708"))),
            Op::Push16(Imm::from(hex!("0102030405060708090a0b0c0d0e0f10"))),
            Op::Push24(Imm::from(hex!(
                "0102030405060708090a0b0c0d0e0f101112131415161718"
            ))),
            Op::Push32(Imm::from(hex!(
                "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20"
            ))),
        ];
        assert_matches!(parse_asm(asm), Ok(e) if e == expected);

        let asm = "push2 0x010203";
        assert_matches!(parse_asm(asm), Err(ParseError::ImmediateTooLarge { .. }));
    }

    #[test]
    fn parse_variable_ops() {
        let asm = r#"
            swap1
            swap4
            swap16
            dup1
            dup4
            dup16
            log0
            log4
        "#;
        let expected = nodes![
            Op::Swap1,
            Op::Swap4,
            Op::Swap16,
            Op::Dup1,
            Op::Dup4,
            Op::Dup16,
            Op::Log0,
            Op::Log4,
        ];
        assert_matches!(parse_asm(asm), Ok(e) if e == expected);
    }

    #[test]
    fn parse_jumpdest_no_label() {
        let asm = "jumpdest";
        let expected = nodes![Op::JumpDest];
        assert_matches!(parse_asm(asm), Ok(e) if e == expected);
    }

    #[test]
    fn parse_jumpdest_label() {
        let asm = "start:\njumpdest";
        let expected = nodes![AbstractOp::Label("start".into()), Op::JumpDest,];
        assert_matches!(parse_asm(asm), Ok(e) if e == expected);
    }

    #[test]
    fn parse_push_label() {
        let asm = r#"
            push2 snake_case
            jumpi
        "#;
        let expected = nodes![Op::Push2(Imm::from("snake_case")), Op::JumpI];
        assert_matches!(parse_asm(asm), Ok(e) if e == expected);
    }

    #[test]
    fn parse_push_op_as_label() {
        let asm = r#"
            push1:
            push1 push1
            jumpi
        "#;
        let expected = nodes![
            AbstractOp::Label("push1".into()),
            Op::Push1(Imm::from("push1")),
            Op::JumpI
        ];
        assert_matches!(parse_asm(asm), Ok(e) if e == expected);
    }

    #[test]
    fn parse_selector() {
        let asm = r#"
            push4 selector("name()")
            push4 selector("balanceOf(address)")
            push4 selector("transfer(address,uint256)")
            push4 selector("approve(address,uint256)")
            push32 selector("transfer(address,uint256)")
        "#;
        let expected = nodes![
            Op::Push4(Imm::from(hex!("06fdde03"))),
            Op::Push4(Imm::from(hex!("70a08231"))),
            Op::Push4(Imm::from(hex!("a9059cbb"))),
            Op::Push4(Imm::from(hex!("095ea7b3"))),
            Op::Push32(Imm::from(hex!(
                "a9059cbb2ab09eb219583f4a59a5d0623ade346d962bcd4e46b11da047c9049b"
            ))),
        ];
        assert_matches!(parse_asm(asm), Ok(e) if e == expected);
    }

    #[test]
    fn parse_selector_with_spaces() {
        let asm = r#"
            push4 selector("name( )")
        "#;
        assert_matches!(parse_asm(asm), Err(ParseError::Lexer { .. }));
    }

    #[test]
    fn parse_include() {
        let asm = format!(
            r#"
            push1 1
            %include("foo.asm")
            push1 2
            "#,
        );
        let expected = nodes![
            Op::Push1(Imm::from(1)),
            Node::Include(PathBuf::from("foo.asm")),
            Op::Push1(Imm::from(2)),
        ];
        assert_matches!(parse_asm(&asm), Ok(e) if e == expected)
    }

    #[test]
    fn parse_include_hex() {
        let asm = format!(
            r#"
            push1 1
            %include_hex("foo.hex")
            push1 2
            "#,
        );
        let expected = nodes![
            Op::Push1(Imm::from(1)),
            Node::IncludeHex(PathBuf::from("foo.hex")),
            Op::Push1(Imm::from(2)),
        ];
        assert_matches!(parse_asm(&asm), Ok(e) if e == expected)
    }

    #[test]
    fn parse_import() {
        let asm = format!(
            r#"
            push1 1
            %import("foo.asm")
            push1 2
            "#,
        );
        let expected = nodes![
            Op::Push1(Imm::from(1)),
            Node::Import(PathBuf::from("foo.asm")),
            Op::Push1(Imm::from(2)),
        ];
        assert_matches!(parse_asm(&asm), Ok(e) if e == expected)
    }

    #[test]
    fn parse_import_extra_argument() {
        let asm = format!(
            r#"
            %import("foo.asm", "bar.asm")
            "#,
        );
        assert!(matches!(
            parse_asm(&asm),
            Err(ParseError::ExtraArgument {
                expected: 1,
                backtrace: _
            })
        ))
    }

    #[test]
    fn parse_import_missing_argument() {
        let asm = format!(
            r#"
            %import()
            "#,
        );
        assert!(matches!(
            parse_asm(&asm),
            Err(ParseError::MissingArgument {
                got: 0,
                expected: 1,
                backtrace: _,
            })
        ))
    }

    #[test]
    fn parse_import_argument_type() {
        let asm = format!(
            r#"
            %import(0x44)
            "#,
        );
        assert_matches!(parse_asm(&asm), Err(ParseError::ArgumentType { .. }))
    }

    #[test]
    fn parse_import_spaces() {
        let asm = format!(
            r#"
            push1 1
            %import( "hello.asm" )
            push1 2
            "#,
        );
        let expected = nodes![
            Op::Push1(Imm::from(1)),
            Node::Import(PathBuf::from("hello.asm")),
            Op::Push1(Imm::from(2)),
        ];
        assert_matches!(parse_asm(&asm), Ok(e) if e == expected)
    }

    #[test]
    fn parse_push_macro_with_label() {
        let asm = format!(
            r#"
            push1 1
            %push( hello )
            push1 2
            "#,
        );
        let expected = nodes![
            Op::Push1(Imm::from(1)),
            AbstractOp::Push("hello".into()),
            Op::Push1(Imm::from(2)),
        ];
        assert_matches!(parse_asm(&asm), Ok(e) if e == expected)
    }
}
