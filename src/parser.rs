use nom::{
    branch::alt,
    bytes::complete::{is_not, tag, take_while, take_while1, take_while_m_n},
    character::complete::{char, digit1, hex_digit1, multispace1, oct_digit1},
    combinator::{all_consuming, cut, map, map_opt, opt, recognize, verify},
    error::{ContextError, ParseError},
    multi::{fold_many0, many0, many0_count},
    sequence::{delimited, pair, preceded, terminated},
    IResult, Parser,
};
use nom::{
    character::complete::{alpha1, alphanumeric1},
    combinator::value,
    error::{convert_error, ErrorKind, FromExternalError, VerboseError},
};
use super::*;

const KEYWORDS: &[&str] = &[
    "def", "fun", "struct", "enum", "mut", "let", "if", "else", "while", "for", "return", "match",
    "True", "False", "Null", "None", "sizeof", "Int", "Float", "Char", "Bool", "Cell", "Never",
    "!",
];

fn is_symbol_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn bin_digit1<'a, E: ParseError<&'a str> + ContextError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, &'a str, E> {
    take_while1(|c: char| c == '0' || c == '1')(input)
}

fn whitespace<'a, E: ParseError<&'a str> + ContextError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, &'a str, E> {
    take_while(|c: char| c.is_whitespace())(input)
}

fn parse_int_literal<'a, E: ParseError<&'a str> + ContextError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, i64, E> {
    // First try to parse hex
    // Check if its negative
    let (input, _) = whitespace(input)?;
    let (input, is_negative) = opt(tag("-"))(input)?;
    let is_negative = is_negative.is_some();
    let (input, _) = whitespace(input)?;

    let (input, result) = alt((
        map(preceded(tag("0x"), hex_digit1), |s: &str| {
            i64::from_str_radix(s, 16).unwrap()
        }),
        // Try octal
        map(preceded(tag("0o"), oct_digit1), |s: &str| {
            i64::from_str_radix(s, 8).unwrap()
        }),
        // Try binary
        map(preceded(tag("0b"), bin_digit1), |s: &str| {
            i64::from_str_radix(s, 2).unwrap()
        }),
        map(digit1, |s: &str| s.parse().unwrap()),
    ))(input)?;

    // let (input, result) = map(digit1, |s: &str| s.parse().unwrap())(input)?;
    if let Some(c) = input.chars().next() {
        if is_symbol_char(c) {
            return Err(nom::Err::Error(E::from_error_kind(input, ErrorKind::Digit)));
        }
    }

    let result = if is_negative { -result } else { result };

    Ok((input, result))
}

fn parse_float_literal<'a, E: ParseError<&'a str> + ContextError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, f64, E> {
    // Parse a signed float
    let (input, _) = whitespace(input)?;
    let (input, is_negative) = opt(tag("-"))(input)?;

    // let (input, result) = map(
    //     pair(digit1, preceded(char('.'), digit1)),
    //     |(a, b): (&str, &str)| format!("{}.{}", a, b).parse::<f64>().unwrap(),
    // )(input)?;

    // // Peek and make sure the next character is not a symbol character
    // if let Some(c) = input.chars().next() {
    //     if is_symbol_char(c) {
    //         return Err(nom::Err::Error(E::from_error_kind(input, ErrorKind::Digit)));
    //     }
    // }

    // Use builtin nom double
    let (input, result) = nom::number::complete::recognize_float(input)?;
    // Try to parse as an integer first
    let result: f64 = if let Ok(_i) = result.parse::<i64>() {
        // Fail
        return Err(nom::Err::Error(E::from_error_kind(input, ErrorKind::Digit)));
    } else {
        result.parse().unwrap()
    };

    if is_negative.is_some() {
        Ok((input, -result))
    } else {
        Ok((input, result))
    }
}

/// Parse a unicode sequence, of the form u{XXXX}, where XXXX is 1 to 6
/// hexadecimal numerals. We will combine this later with parse_escaped_char
/// to parse sequences like \u{00AC}.
fn parse_unicode<'a, E>(input: &'a str) -> IResult<&'a str, char, E>
where
    E: ParseError<&'a str>,
{
    // `take_while_m_n` parses between `m` and `n` bytes (inclusive) that match
    // a predicate. `parse_hex` here parses between 1 and 6 hexadecimal numerals.
    let parse_hex = take_while_m_n(1, 6, |c: char| c.is_ascii_hexdigit());

    // `preceded` takes a prefix parser, and if it succeeds, returns the result
    // of the body parser. In this case, it parses u{XXXX}.
    let parse_delimited_hex = preceded(
        char('u'),
        // `delimited` is like `preceded`, but it parses both a prefix and a suffix.
        // It returns the result of the middle parser. In this case, it parses
        // {XXXX}, where XXXX is 1 to 6 hex numerals, and returns XXXX
        delimited(char('{'), parse_hex, char('}')),
    );

    // `map_res` takes the result of a parser and applies a function that returns
    // a Result. In this case we take the hex bytes from parse_hex and attempt to
    // convert them to a u32.
    let parse_u32 = map(parse_delimited_hex, move |hex| {
        u32::from_str_radix(hex, 16).unwrap()
    });

    // map_opt is like map_res, but it takes an Option instead of a Result. If
    // the function returns None, map_opt returns an error. In this case, because
    // not all u32 values are valid unicode code points, we have to fallibly
    // convert to char with from_u32.
    map_opt(parse_u32, std::char::from_u32).parse(input)
}

/// Parse an escaped character: \n, \t, \r, \u{00AC}, etc.
fn parse_escaped_char<'a, E>(input: &'a str) -> IResult<&'a str, char, E>
where
    E: ParseError<&'a str>,
{
    preceded(
        char('\\'),
        // `alt` tries each parser in sequence, returning the result of
        // the first successful match
        alt((
            parse_unicode,
            // The `value` parser returns a fixed value (the first argument) if its
            // parser (the second argument) succeeds. In these cases, it looks for
            // the marker characters (n, r, t, etc) and returns the matching
            // character (\n, \r, \t, etc).
            value('\0', char('0')),
            value('\n', char('n')),
            value('\r', char('r')),
            value('\t', char('t')),
            value('\u{08}', char('b')),
            value('\u{0C}', char('f')),
            value('\\', char('\\')),
            value('/', char('/')),
            value('"', char('"')),
            value('\'', char('\'')),
            // Parse an \x followed by two hex digits, and convert it to a char
            map(
                preceded(char('x'), take_while_m_n(2, 2, |c: char| c.is_ascii_hexdigit())),
                |hex| u8::from_str_radix(hex, 16).unwrap() as char,
            ),
            // Parse an \u followed by four hex digits, and convert it to a char
            // map(
            //     preceded(char('u'), take_while_m_n(4, 4, |c: char| c.is_ascii_hexdigit())),
            //     |hex| char::from_u32(u32::from_str_radix(hex, 16).unwrap()).unwrap(),
            // ),
        )),
    )
    .parse(input)
}

/// Parse a backslash, followed by any amount of whitespace. This is used later
/// to discard any escaped whitespace.
fn parse_escaped_whitespace<'a, E: ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, &'a str, E> {
    preceded(char('\\'), multispace1).parse(input)
}

/// Parse a non-empty block of text that doesn't include \ or "
fn parse_literal_intermediate<'a, E: ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, &'a str, E> {
    // `is_not` parses a string of 0 or more characters that aren't one of the
    // given characters.
    let not_quote_slash = is_not("\"\\");

    // `verify` runs a parser, then runs a verification function on the output of
    // the parser. The verification function accepts out output only if it
    // returns true. In this case, we want to ensure that the output of is_not
    // is non-empty.
    verify(not_quote_slash, |s: &str| !s.is_empty()).parse(input)
}

/// Parse a non-empty block of text that doesn't include \ or "
fn parse_literal_intermediate_char<'a, E: ParseError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, &'a str, E> {
    // `is_not` parses a string of 0 or more characters that aren't one of the
    // given characters.
    let not_quote_slash = is_not("\'\\");

    // `verify` runs a parser, then runs a verification function on the output of
    // the parser. The verification function accepts out output only if it
    // returns true. In this case, we want to ensure that the output of is_not
    // is non-empty.
    verify(not_quote_slash, |s: &str| !s.is_empty()).parse(input)
}

/// A string fragment contains a fragment of a string being parsed: either
/// a non-empty Literal (a series of non-escaped characters), a single
/// parsed escaped character, or a block of escaped whitespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StringFragment<'a> {
    Literal(&'a str),
    EscapedChar(char),
    EscapedWS,
}

/// Combine parse_literal, parse_escaped_whitespace, and parse_escaped_char
/// into a StringFragment.
fn parse_fragment_str<'a, E>(input: &'a str) -> IResult<&'a str, StringFragment<'a>, E>
where
    E: ParseError<&'a str>,
{
    alt((
        // The `map` combinator runs a parser, then applies a function to the output
        // of that parser.
        map(parse_literal_intermediate, StringFragment::Literal),
        map(parse_escaped_char, StringFragment::EscapedChar),
        value(StringFragment::EscapedWS, parse_escaped_whitespace),
    ))
    .parse(input)
}

fn parse_fragment_char<'a, E>(input: &'a str) -> IResult<&'a str, StringFragment<'a>, E>
where
    E: ParseError<&'a str>,
{
    alt((
        // The `map` combinator runs a parser, then applies a function to the output
        // of that parser.
        map(parse_literal_intermediate_char, StringFragment::Literal),
        map(parse_escaped_char, StringFragment::EscapedChar),
        value(StringFragment::EscapedWS, parse_escaped_whitespace),
    ))
    .parse(input)
}

/// Parse a string. Use a loop of parse_fragment and push all of the fragments
/// into an output string.
fn parse_string<'a, E>(input: &'a str) -> IResult<&'a str, String, E>
where
    E: ParseError<&'a str> + FromExternalError<&'a str, std::num::ParseIntError>,
{
    // fold is the equivalent of iterator::fold. It runs a parser in a loop,
    // and for each output value, calls a folding function on each output value.
    let build_string = fold_many0(
        // Our parser function – parses a single string fragment
        parse_fragment_str,
        // Our init value, an empty string
        String::new,
        // Our folding function. For each fragment, append the fragment to the
        // string.
        |mut string, fragment| {
            match fragment {
                StringFragment::Literal(s) => string.push_str(s),
                StringFragment::EscapedChar(c) => string.push(c),
                StringFragment::EscapedWS => {}
            }
            string
        },
    );

    // Finally, parse the string. Note that, if `build_string` could accept a raw
    // " character, the closing delimiter " would never match. When using
    // `delimited` with a looping parser (like fold), be sure that the
    // loop won't accidentally match your closing delimiter!
    delimited(char('"'), build_string, char('"')).parse(input)
}

fn parse_char_literal<'a, E: ParseError<&'a str> + ContextError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, char, E> {
    let (input, _) = whitespace(input)?;
    // fold is the equivalent of iterator::fold. It runs a parser in a loop,
    // and for each output value, calls a folding function on each output value.
    let build_string = fold_many0(
        // Our parser function – parses a single string fragment
        parse_fragment_char,
        // Our init value, an empty string
        String::new,
        // Our folding function. For each fragment, append the fragment to the
        // string.
        |mut string, fragment| {
            match fragment {
                StringFragment::Literal(s) => string.push_str(s),
                StringFragment::EscapedChar(c) => string.push(c),
                StringFragment::EscapedWS => {}
            }
            string
        },
    );

    // Finally, parse the string. Note that, if `build_string` could accept a raw
    // " character, the closing delimiter " would never match. When using
    // `delimited` with a looping parser (like fold), be sure that the
    // loop won't accidentally match your closing delimiter!
    let (input, result) = delimited(char('\''), cut(build_string), cut(char('\''))).parse(input)?;
    if result.len() != 1 {
        return Err(nom::Err::Error(E::from_error_kind(input, ErrorKind::Digit)));
    }
    Ok((input, result.chars().next().unwrap()))
}

fn parse_string_literal<'a, E: ParseError<&'a str> + ContextError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, String, E> {
    // map(context(
    //   "string",
    //   alt((
    //         // preceded(char('\''), cut(terminated(parse_inner_str_single, char('\'')))),
    //         preceded(char('"'), cut(terminated(parse_inner_str_double, char('"')))),
    //   )),
    // ), |s| s.to_string())(input)

    if let Ok((input, s)) = parse_string::<VerboseError<&str>>(input) {
        Ok((input, s))
    } else {
        Err(nom::Err::Error(E::from_error_kind(input, ErrorKind::Digit)))
    }
}

fn parse_bool<'a, E: ParseError<&'a str> + ContextError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, bool, E> {
    let (input, result) = alt((value(true, tag("true")), value(false, tag("false"))))(input)?;

    // Peek and make sure the next character is not a symbol character
    if let Some(c) = input.chars().next() {
        if is_symbol_char(c) {
            return Err(nom::Err::Error(E::from_error_kind(input, ErrorKind::Digit)));
        }
    }

    Ok((input, result))
}

fn parse_symbol<'a, E: ParseError<&'a str> + ContextError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, &'a str, E> {
    let (input, _) = whitespace(input)?;
    // recognize(all_consuming(pair(
    //     verify(anychar, |&c| c.is_lowercase() || c == '_'),
    //     many0_count(preceded(opt(char('_')), alphanumeric1)),
    // )))(input)
    let (input, result) = recognize(pair(
        alt((alpha1, tag("_"))),
        many0_count(alt((alphanumeric1, tag("_")))),
    ))(input)?;
    if KEYWORDS.contains(&result) {
        return Err(nom::Err::Error(E::from_error_kind(input, ErrorKind::Tag)));
    }
    Ok((input, result))
}

fn parse_expr<'a, E: ParseError<&'a str> + ContextError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, Expr, E> {
    let (input, _) = whitespace(input)?;
    let (input, result) = alt((parse_app, parse_expr_atom))(input)?;
    let (input, _) = whitespace(input)?;
    Ok((input, result))
}

fn parse_array<'a, E: ParseError<&'a str> + ContextError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, Expr, E> {
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("[")(input)?;
    let (input, _) = whitespace(input)?;
    let (input, mut elems) = cut(many0(terminated(parse_expr, delimited(whitespace, char(','), whitespace))))(input)?;
    // Check if there is a trailing comma
    let (input, last) = opt(preceded(whitespace, parse_expr))(input)?;
    if let Some(last) = last {
        elems.push(last);
    }
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("]")(input)?;
    Ok((input, Expr::Array(elems)))
}

fn parse_expr_atom<'a, E: ParseError<&'a str> + ContextError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, Expr, E> {
    let (input, _) = whitespace(input)?;
    let (input, result) = alt((
        map(parse_float_literal, Expr::Float),
        map(parse_int_literal, Expr::Int),
        map(parse_char_literal, Expr::Char),
        map(parse_bool, Expr::Bool),
        map(parse_symbol, |s| Expr::Var(s.into())),
        parse_array,
        map(parse_string_literal, |s| {
            // Convert the string to a vector of bytes
            let bytes = s.as_bytes().to_vec();
            // Now concat the bytes into i64s (8 bytes at a time)
            let mut result = Vec::new();
            for chunk in bytes.chunks(8) {
                let mut bytes = [0; 8];
                for (i, &byte) in chunk.iter().enumerate() {
                    bytes[i] = byte;
                }
                result.push(i64::from_ne_bytes(bytes));
            }
            result.push(0);
            Expr::Array(result.into_iter().map(Expr::Int).collect())
        }),
        delimited(
            char('('),
            preceded(whitespace, parse_expr),
            preceded(whitespace, char(')')),
        ),
        parse_ref,
        parse_if_expr
    ))(input)?;
    let (input, _) = whitespace(input)?;
    Ok((input, result))
}

fn parse_ref<'a, E: ParseError<&'a str> + ContextError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, Expr, E> {
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("&")(input)?;
    let (input, _) = whitespace(input)?;
    let (input, result) = parse_symbol(input)?;
    Ok((input, ref_(result)))
}

fn parse_app<'a, E: ParseError<&'a str> + ContextError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, Expr, E> {
    let (input, _) = whitespace(input)?;
    let (input, f) = parse_expr_atom(input)?;
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("(")(input)?;
    let (input, mut args) = many0(terminated(
        preceded(whitespace, parse_expr),
        preceded(whitespace, char(',')),
    ))(input)?;
    // Check if there is a trailing comma
    let (input, last) = opt(preceded(whitespace, parse_expr))(input)?;
    if let Some(last) = last {
        args.push(last);
    }
    let (input, _) = whitespace(input)?;
    let (input, _) = tag(")")(input)?;
    Ok((input, app(f, args)))
}

fn parse_if_expr<'a, E: ParseError<&'a str> + ContextError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, Expr, E> {
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("if")(input)?;
    let (input, cond) = parse_expr(input)?;
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("{")(input)?;
    let (input, _) = whitespace(input)?;
    let (input, then_block) = parse_expr(input)?;
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("}")(input)?;
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("else")(input)?;
    let (input, _) = whitespace(input)?;

    let (input, else_block) = alt((
        parse_if_expr,
        delimited(tag("{"), parse_expr, tag("}"))
    ))(input)?;
    let (input, _) = whitespace(input)?;
    

    // let (input, _) = whitespace(input)?;
    // let (input, _) = tag("{")(input)?;
    // let (input, _) = whitespace(input)?;
    // let (input, else_block) = parse_expr(input)?;
    // let (input, _) = whitespace(input)?;
    // let (input, _) = tag("}")(input)?;
    // let (input, _) = whitespace(input)?;

    Ok((input, if_expr(cond, then_block, else_block)))
}

fn parse_if_stmt<'a, E: ParseError<&'a str> + ContextError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, Stmt, E> {
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("if")(input)?;
    let (input, _) = whitespace(input)?;
    let (input, cond) = cut(parse_expr)(input)?;
    let (input, _) = whitespace(input)?;
    let (input, then_block) = cut(parse_block)(input)?;
    let (input, _) = whitespace(input)?;
    let (input, has_else) = opt(tag("else"))(input)?;
    if has_else.is_none() {
        return Ok((input, if_(cond, Stmt::Block(then_block), Stmt::Block(vec![]))));
    }

    let (input, _) = whitespace(input)?;

    let (input, else_block) = cut(alt((
        map(parse_if_stmt, |stmt| vec![stmt]),
        parse_block
    )))(input)?;
    let (input, _) = whitespace(input)?;

    Ok((input, if_(cond, Stmt::Block(then_block), Stmt::Block(else_block))))
}

fn parse_while_stmt<'a, E: ParseError<&'a str> + ContextError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, Stmt, E> {
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("while")(input)?;
    let (input, cond) = parse_expr(input)?;
    let (input, _) = whitespace(input)?;
    let (input, block) = parse_block(input)?;
    let (input, _) = whitespace(input)?;

    Ok((input, while_(cond, Stmt::Block(block))))
}

fn parse_block<'a, E: ParseError<&'a str> + ContextError<&'a str>>(input: &'a str) -> IResult<&'a str, Vec<Stmt>, E> {
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("{")(input)?;
    let (input, _) = whitespace(input)?;
    let (input, stmts) = many0(parse_stmt)(input)?;
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("}")(input)?;
    Ok((input, stmts))
}

fn parse_break_stmt<'a, E: ParseError<&'a str> + ContextError<&'a str>>(input: &'a str) -> IResult<&'a str, Stmt, E> {
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("break")(input)?;
    let (input, _) = whitespace(input)?;
    let (input, _) = tag(";")(input)?;
    Ok((input, Stmt::Break))
}

fn parse_continue_stmt<'a, E: ParseError<&'a str> + ContextError<&'a str>>(
    input: &'a str,
) -> IResult<&'a str, Stmt, E> {
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("continue")(input)?;
    let (input, _) = whitespace(input)?;
    let (input, _) = tag(";")(input)?;
    Ok((input, Stmt::Continue))
}

fn parse_stmt<'a, E: ParseError<&'a str> + ContextError<&'a str>>(input: &'a str) -> IResult<&'a str, Stmt, E> {
    let (input, _) = whitespace(input)?;
    let start = input.len();
    let (input, stmt) = alt((
        parse_break_stmt,
        parse_continue_stmt,
        parse_return_stmt,
        parse_while_stmt,
        parse_if_stmt,
        parse_extern_proc,
        parse_proc,
        parse_let,
        parse_let_static,
        parse_assign,
        parse_stmt_expr,
        map(parse_block, Stmt::Block),
    ))(input)?;
    let end = input.len();
    let length = start - end;
    let metadata = get_loc(input, length);
    let (input, _) = whitespace(input)?;
    Ok((input, stmt.annotate(metadata)))
}

fn parse_stmts<'a, E: ParseError<&'a str> + ContextError<&'a str>>(input: &'a str) -> IResult<&'a str, Vec<Stmt>, E> {
    let (input, _) = whitespace(input)?;
    let (input, stmts) = many0(parse_stmt)(input)?;
    let (input, _) = whitespace(input)?;
    Ok((input, stmts))
}

fn parse_extern_proc<'a, E: ParseError<&'a str> + ContextError<&'a str>>(input: &'a str) -> IResult<&'a str, Stmt, E> {
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("extern")(input)?;
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("fun")(input)?;
    let (input, _) = whitespace(input)?;
    let (input, name) = parse_symbol(input)?;
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("(")(input)?;
    let (input, mut args) = many0(terminated(
        preceded(whitespace, parse_symbol),
        preceded(whitespace, char(',')),
    ))(input)?;
    // Check if there is a trailing comma
    let (input, last) = opt(preceded(whitespace, parse_symbol))(input)?;
    if let Some(last) = last {
        args.push(last);
    }
    let (input, _) = tag(")")(input)?;
    let (input, _) = whitespace(input)?;
    let (input, body) = alt((
        value(None, tag(";")),
        cut(map(parse_block, |stmts| Some(Stmt::Block(stmts)))),
    ))(input)?;
    let (input, _) = whitespace(input)?;
    Ok((input, extern_proc(name, args, body)))
}

fn parse_proc<'a, E: ParseError<&'a str> + ContextError<&'a str>>(input: &'a str) -> IResult<&'a str, Stmt, E> {
    // Parse an optional export keyword, followed by a function definition
    // let (input, _) = whitespace(input)?;
    // let (input, export) = opt(tag("export"))(input)?;
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("fun")(input)?;
    let (input, _) = whitespace(input)?;
    let (input, name) = parse_symbol(input)?;
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("(")(input)?;
    let (input, mut args) = many0(terminated(
        preceded(whitespace, parse_symbol),
        preceded(whitespace, char(',')),
    ))(input)?;
    // Check if there is a trailing comma
    let (input, last) = opt(preceded(whitespace, parse_symbol))(input)?;
    if let Some(last) = last {
        args.push(last);
    }
    let (input, _) = tag(")")(input)?;
    let (input, _) = whitespace(input)?;
    let (input, block) = cut(parse_block)(input)?;
    // Ok((input, proc(export.is_some(), name, args, Stmt::Block(block))))
    Ok((input, proc(name, args, Stmt::Block(block))))
}

fn parse_let<'a, E: ParseError<&'a str> + ContextError<&'a str>>(input: &'a str) -> IResult<&'a str, Stmt, E> {
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("let")(input)?;
    let (input, _) = whitespace(input)?;
    let (input, name) = parse_symbol(input)?;
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("=")(input)?;
    let (input, _) = whitespace(input)?;
    let (input, value) = parse_expr(input)?;
    let (input, _) = whitespace(input)?;
    let (input, _) = tag(";")(input)?;
    Ok((input, let_var(name, value)))
}

fn parse_let_static<'a, E: ParseError<&'a str> + ContextError<&'a str>>(input: &'a str) -> IResult<&'a str, Stmt, E> {
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("let")(input)?;
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("static")(input)?;
    let (input, _) = whitespace(input)?;
    let (input, name) = parse_symbol(input)?;
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("=")(input)?;
    let (input, _) = whitespace(input)?;
    let (input, value) = parse_expr(input)?;
    let (input, _) = whitespace(input)?;
    let (input, _) = tag(";")(input)?;
    Ok((input, let_static(name, value)))
}

fn parse_assign_var<'a, E: ParseError<&'a str> + ContextError<&'a str>>(input: &'a str) -> IResult<&'a str, Stmt, E> {
    let (input, _) = whitespace(input)?;
    let (input, name) = parse_symbol(input)?;
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("=")(input)?;
    let (input, _) = whitespace(input)?;
    let (input, value) = parse_expr(input)?;
    let (input, _) = whitespace(input)?;
    let (input, _) = tag(";")(input)?;
    Ok((input, assign_var(name, value)))
}

fn parse_assign_value<'a, E: ParseError<&'a str> + ContextError<&'a str>>(input: &'a str) -> IResult<&'a str, Stmt, E> {
    let (input, _) = whitespace(input)?;
    let (input, _) = opt(tag("*"))(input)?;
    let (input, _) = whitespace(input)?;
    let (input, name) = parse_expr(input)?;
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("=")(input)?;
    let (input, _) = whitespace(input)?;
    let (input, value) = parse_expr(input)?;
    let (input, _) = whitespace(input)?;
    let (input, _) = tag(";")(input)?;
    Ok((input, assign_ref(name, value)))
}

fn parse_assign<'a, E: ParseError<&'a str> + ContextError<&'a str>>(input: &'a str) -> IResult<&'a str, Stmt, E> {
    let (input, _) = whitespace(input)?;
    let (input, stmt) = alt((parse_assign_var, parse_assign_value))(input)?;
    Ok((input, stmt))
}

fn parse_stmt_expr<'a, E: ParseError<&'a str> + ContextError<&'a str>>(input: &'a str) -> IResult<&'a str, Stmt, E> {
    let (input, _) = whitespace(input)?;
    let (input, expr) = parse_expr(input)?;
    let (input, _) = whitespace(input)?;
    let (input, _) = tag(";")(input)?;
    Ok((input, stmt(expr)))
}

fn parse_return_stmt<'a, E: ParseError<&'a str> + ContextError<&'a str>>(input: &'a str) -> IResult<&'a str, Stmt, E> {
    let (input, _) = whitespace(input)?;
    let (input, _) = tag("return")(input)?;
    let (input, _) = whitespace(input)?;
    let (input, expr) = parse_expr(input)?;
    let (input, _) = whitespace(input)?;
    let (input, _) = tag(";")(input)?;
    Ok((input, return_(expr)))
}

lazy_static! {
    static ref CURRENT_PROGRAM: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
}

fn get_loc(remaining_input: &str, length: usize) -> SourceCodeLocation {
    let current_program = CURRENT_PROGRAM.lock().unwrap();
    let mut line = 0;
    let mut column = 0;
    for c in current_program.chars().take(current_program.len() - remaining_input.len()) {
        if c == '\n' {
            line += 1;
            column = 0;
        } else {
            column += 1;
        }
    }
    SourceCodeLocation {
        line,
        column,
        length
    }
}

pub fn parse(input: &str) -> anyhow::Result<Stmt> {
    // First, strip comments
    *CURRENT_PROGRAM.lock().unwrap() = input.to_string();

    let input = input
        .lines()
        .map(|line| {
            if let Some(pos) = line.find("//") {
                &line[..pos]
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    
    parse_helper(&input)
}

fn parse_helper(input: &str) -> anyhow::Result<Stmt> {
    let (_, stmts) = all_consuming(parse_stmts)(input).map_err(|e| {
        match e {
            nom::Err::Error(e) | nom::Err::Failure(e) => {
                convert_error(input, e)
            }
            nom::Err::Incomplete(_) => unreachable!(),
        }
    }).map_err(anyhow::Error::msg)?;
    Ok(Stmt::Block(stmts))
}