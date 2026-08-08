//! Bounded baseline ShapeSheet evaluation. Unsupported formulas never use cached values.

use std::collections::{BTreeMap, HashSet};

use thiserror::Error;
use vsdx_parse::ParseLimits;
use vsdx_resolve::{Lookup, ResolvedShape};

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Number(f64, Unit),
    String(String),
    Reference(String),
    Unary(Box<Expr>),
    Binary(Box<Expr>, Op, Box<Expr>),
    Call(String, Vec<Expr>),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Number,
    Bool,
    Inches,
    Radians,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Value {
    pub number: f64,
    pub unit: Unit,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Evaluation {
    Evaluated(Value),
    Unsupported(String),
    Error(Diagnostic),
}
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{message}")]
pub struct Diagnostic {
    pub message: String,
}

/// A reference provider normally backed by a `vsdx_resolve::ResolvedShape` so inheritance
/// is applied before formulas are evaluated.
pub trait References {
    fn formula(&self, name: &str) -> Option<&str>;
}
impl References for ResolvedShape {
    fn formula(&self, name: &str) -> Option<&str> {
        let direct = self.cells.get(name);
        let sectioned = name.split_once('.').and_then(|(section, cell)| {
            self.sections
                .get(section)
                .and_then(|s| s.rows.values().find_map(|r| r.cells.get(cell)))
        });
        match direct.or(sectioned) {
            Some(Lookup::Found(value)) => value.cell.formula.as_deref(),
            _ => None,
        }
    }
}
impl References for BTreeMap<String, String> {
    fn formula(&self, name: &str) -> Option<&str> {
        self.get(name).map(String::as_str)
    }
}

pub fn parse(input: &str, limits: &ParseLimits) -> Result<Expr, Diagnostic> {
    if input.trim().eq_ignore_ascii_case("No Formula") {
        return Ok(Expr::Call("No Formula".into(), Vec::new()));
    }
    Parser::new(input, limits.max_formula_depth).parse()
}
pub fn evaluate(input: &str, refs: &impl References, limits: &ParseLimits) -> Evaluation {
    match parse(input, limits) {
        Ok(expr) => Engine {
            refs,
            limits,
            active: HashSet::new(),
        }
        .expr(&expr, 0),
        Err(error) => Evaluation::Error(error),
    }
}

struct Engine<'a, R> {
    refs: &'a R,
    limits: &'a ParseLimits,
    active: HashSet<String>,
}
impl<R: References> Engine<'_, R> {
    fn expr(&mut self, expr: &Expr, depth: usize) -> Evaluation {
        if depth > self.limits.max_formula_depth {
            return err("formula depth limit exceeded");
        }
        match expr {
            Expr::Number(n, u) => good(*n, *u),
            Expr::String(_) => unsupported("string values are not display numbers"),
            Expr::Reference(name) => {
                if !self.active.insert(name.clone()) {
                    return err(format!("reference cycle at {name}"));
                }
                let result = match self.refs.formula(name) {
                    Some(formula) => match parse(formula, self.limits) {
                        Ok(e) => self.expr(&e, depth + 1),
                        Err(e) => Evaluation::Error(e),
                    },
                    None => err(format!("unresolved reference {name}")),
                };
                self.active.remove(name);
                result
            }
            Expr::Unary(v) => match value(self.expr(v, depth + 1)) {
                Ok(v) => good(-v.number, v.unit),
                Err(r) => r,
            },
            Expr::Binary(a, op, b) => self.binary(a, *op, b, depth + 1),
            Expr::Call(name, args) => self.call(name, args, depth + 1),
        }
    }
    fn binary(&mut self, a: &Expr, op: Op, b: &Expr, d: usize) -> Evaluation {
        let a = match value(self.expr(a, d)) {
            Ok(v) => v,
            Err(r) => return r,
        };
        let b = match value(self.expr(b, d)) {
            Ok(v) => v,
            Err(r) => return r,
        };
        match op {
            Op::Add | Op::Sub => {
                if a.unit != b.unit {
                    return err("incompatible units");
                }
                good(
                    if op == Op::Add {
                        a.number + b.number
                    } else {
                        a.number - b.number
                    },
                    a.unit,
                )
            }
            Op::Mul => {
                if a.unit != Unit::Number && b.unit != Unit::Number {
                    err("cannot multiply dimensional values")
                } else {
                    good(
                        a.number * b.number,
                        if a.unit == Unit::Number {
                            b.unit
                        } else {
                            a.unit
                        },
                    )
                }
            }
            Op::Div => {
                if b.number == 0.0 {
                    return err("division by zero");
                };
                if b.unit != Unit::Number && a.unit != b.unit {
                    return err("incompatible units");
                };
                good(
                    a.number / b.number,
                    if a.unit == b.unit {
                        Unit::Number
                    } else {
                        a.unit
                    },
                )
            }
            Op::Pow => {
                if b.unit != Unit::Number {
                    return err("exponent must be dimensionless");
                };
                good(a.number.powf(b.number), a.unit)
            }
            Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                if a.unit != b.unit {
                    return err("incompatible units");
                };
                let x = match op {
                    Op::Eq => a.number == b.number,
                    Op::Ne => a.number != b.number,
                    Op::Lt => a.number < b.number,
                    Op::Le => a.number <= b.number,
                    Op::Gt => a.number > b.number,
                    Op::Ge => a.number >= b.number,
                    _ => false,
                };
                good(if x { 1. } else { 0. }, Unit::Bool)
            }
        }
    }
    fn call(&mut self, name: &str, args: &[Expr], d: usize) -> Evaluation {
        let upper = name.to_ascii_uppercase();
        if matches!(
            upper.as_str(),
            "GUARD" | "SETATREF" | "SETATREFEXPR" | "SETATREFEVAL" | "DEPENDSON"
        ) {
            return unsupported(format!("{upper} is outside the phase-4 evaluator"));
        }
        let vals: Result<Vec<_>, _> = args.iter().map(|a| value(self.expr(a, d))).collect();
        let vals = match vals {
            Ok(v) => v,
            Err(r) => return r,
        };
        let one = || vals.first().copied().ok_or_else(|| err("missing argument"));
        let same = || {
            if vals.iter().map(|v| v.unit).all(|u| u == vals[0].unit) {
                Ok(())
            } else {
                Err(err("incompatible units"))
            }
        };
        match upper.as_str() {
            "IF" if vals.len() == 3 => {
                if vals[0].number != 0. {
                    good(vals[1].number, vals[1].unit)
                } else {
                    good(vals[2].number, vals[2].unit)
                }
            }
            "AND" => good(
                if vals.iter().all(|v| v.number != 0.) {
                    1.
                } else {
                    0.
                },
                Unit::Bool,
            ),
            "OR" => good(
                if vals.iter().any(|v| v.number != 0.) {
                    1.
                } else {
                    0.
                },
                Unit::Bool,
            ),
            "NOT" => one().map_or_else(
                |r| r,
                |v| good(if v.number == 0. { 1. } else { 0. }, Unit::Bool),
            ),
            "MIN" | "MAX" | "SUM" => {
                if vals.is_empty() {
                    return err("missing argument");
                }
                if let Err(r) = same() {
                    return r;
                }
                let n = if upper == "SUM" {
                    vals.iter().map(|v| v.number).sum()
                } else if upper == "MIN" {
                    vals.iter().map(|v| v.number).fold(f64::INFINITY, f64::min)
                } else {
                    vals.iter()
                        .map(|v| v.number)
                        .fold(f64::NEG_INFINITY, f64::max)
                };
                good(n, vals[0].unit)
            }
            "ABS" => one().map_or_else(|r| r, |v| good(v.number.abs(), v.unit)),
            "INT" => one().map_or_else(|r| r, |v| good(v.number.floor(), v.unit)),
            "TRUNC" => one().map_or_else(|r| r, |v| good(v.number.trunc(), v.unit)),
            "SIGN" => one().map_or_else(|r| r, |v| good(v.number.signum(), Unit::Number)),
            "ROUND" => one().map_or_else(|r| r, |v| good((v.number + 0.5).floor(), v.unit)),
            "CEILING" => one().map_or_else(|r| r, |v| good(v.number.ceil(), v.unit)),
            "FLOOR" => one().map_or_else(|r| r, |v| good(v.number.floor(), v.unit)),
            "SQRT" => one().map_or_else(
                |r| r,
                |v| {
                    if v.number < 0. {
                        err("square root of negative number")
                    } else {
                        good(v.number.sqrt(), v.unit)
                    }
                },
            ),
            "SIN" | "COS" | "TAN" => one().map_or_else(
                |r| r,
                |v| {
                    if v.unit != Unit::Radians {
                        err("trigonometric argument must be an angle")
                    } else {
                        good(
                            if upper == "SIN" {
                                v.number.sin()
                            } else if upper == "COS" {
                                v.number.cos()
                            } else {
                                v.number.tan()
                            },
                            Unit::Number,
                        )
                    }
                },
            ),
            "ATAN2" if vals.len() == 2 => {
                if vals[0].unit != vals[1].unit {
                    return err("incompatible units");
                }
                good(vals[0].number.atan2(vals[1].number), Unit::Radians)
            }
            "PI" if vals.is_empty() => good(std::f64::consts::PI, Unit::Number),
            "MOD" if vals.len() == 2 => {
                if vals[0].unit != vals[1].unit {
                    return err("incompatible units");
                }
                if vals[1].number == 0. {
                    err("division by zero")
                } else {
                    good(vals[0].number.rem_euclid(vals[1].number), vals[0].unit)
                }
            }
            _ => unsupported(format!("unsupported function {upper}")),
        }
    }
}
fn good(number: f64, unit: Unit) -> Evaluation {
    if number.is_finite() {
        Evaluation::Evaluated(Value { number, unit })
    } else {
        err("non-finite result")
    }
}
fn err(message: impl Into<String>) -> Evaluation {
    Evaluation::Error(Diagnostic {
        message: message.into(),
    })
}
fn unsupported(message: impl Into<String>) -> Evaluation {
    Evaluation::Unsupported(message.into())
}
fn value(result: Evaluation) -> Result<Value, Evaluation> {
    match result {
        Evaluation::Evaluated(v) => Ok(v),
        x => Err(x),
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Number(f64, Unit),
    String(String),
    Ident(String),
    Op(Op),
    L,
    R,
    Comma,
    End,
}
struct Parser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    current: Tok,
    depth: usize,
    max: usize,
}
impl<'a> Parser<'a> {
    fn new(s: &'a str, max: usize) -> Self {
        let mut p = Self {
            chars: s.chars().peekable(),
            current: Tok::End,
            depth: 0,
            max,
        };
        p.next();
        p
    }
    fn parse(mut self) -> Result<Expr, Diagnostic> {
        let e = self.cmp()?;
        if self.current != Tok::End {
            return Err(Diagnostic {
                message: "unexpected token".into(),
            });
        }
        Ok(e)
    }
    fn next(&mut self) {
        self.current = self.lex()
    }
    fn lex(&mut self) -> Tok {
        while self.chars.peek().is_some_and(|c| c.is_whitespace()) {
            self.chars.next();
        }
        let Some(c) = self.chars.next() else {
            return Tok::End;
        };
        match c {
            '(' => Tok::L,
            ')' => Tok::R,
            ',' => Tok::Comma,
            '+' => Tok::Op(Op::Add),
            '-' => Tok::Op(Op::Sub),
            '*' => Tok::Op(Op::Mul),
            '/' => Tok::Op(Op::Div),
            '^' => Tok::Op(Op::Pow),
            '=' => Tok::Op(Op::Eq),
            '<' => {
                if self.chars.next_if_eq(&'=').is_some() {
                    Tok::Op(Op::Le)
                } else if self.chars.next_if_eq(&'>').is_some() {
                    Tok::Op(Op::Ne)
                } else {
                    Tok::Op(Op::Lt)
                }
            }
            '>' => {
                if self.chars.next_if_eq(&'=').is_some() {
                    Tok::Op(Op::Ge)
                } else {
                    Tok::Op(Op::Gt)
                }
            }
            '"' => {
                let s = self.chars.by_ref().take_while(|x| *x != '"').collect();
                Tok::String(s)
            }
            x if x.is_ascii_digit() || x == '.' => {
                let mut s = x.to_string();
                while self
                    .chars
                    .peek()
                    .is_some_and(|x| x.is_ascii_digit() || *x == '.' || *x == 'e' || *x == 'E')
                {
                    s.push(self.chars.next().unwrap());
                }
                let mut n = s.parse().unwrap_or(f64::NAN);
                while self.chars.peek().is_some_and(|x| x.is_whitespace()) {
                    self.chars.next();
                }
                let mut u = String::new();
                while self.chars.peek().is_some_and(|x| x.is_ascii_alphabetic()) {
                    u.push(self.chars.next().unwrap());
                }
                let (unit, scale) = unit(&u).unwrap_or((Unit::Number, 1.));
                n *= scale;
                Tok::Number(n, unit)
            }
            x => {
                let mut s = x.to_string();
                while self
                    .chars
                    .peek()
                    .is_some_and(|x| x.is_ascii_alphanumeric() || matches!(*x, '.' | '!' | '_'))
                {
                    s.push(self.chars.next().unwrap());
                }
                Tok::Ident(s)
            }
        }
    }
    fn cmp(&mut self) -> Result<Expr, Diagnostic> {
        let mut x = self.add()?;
        while let Tok::Op(op @ (Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge)) = self.current
        {
            self.next();
            x = Expr::Binary(Box::new(x), op, Box::new(self.add()?));
        }
        Ok(x)
    }
    fn add(&mut self) -> Result<Expr, Diagnostic> {
        let mut x = self.mul()?;
        while let Tok::Op(op @ (Op::Add | Op::Sub)) = self.current {
            self.next();
            x = Expr::Binary(Box::new(x), op, Box::new(self.mul()?));
        }
        Ok(x)
    }
    fn mul(&mut self) -> Result<Expr, Diagnostic> {
        let mut x = self.pow()?;
        while let Tok::Op(op @ (Op::Mul | Op::Div)) = self.current {
            self.next();
            x = Expr::Binary(Box::new(x), op, Box::new(self.pow()?));
        }
        Ok(x)
    }
    fn pow(&mut self) -> Result<Expr, Diagnostic> {
        let x = self.primary()?;
        if self.current == Tok::Op(Op::Pow) {
            self.next();
            Ok(Expr::Binary(Box::new(x), Op::Pow, Box::new(self.pow()?)))
        } else {
            Ok(x)
        }
    }
    fn primary(&mut self) -> Result<Expr, Diagnostic> {
        if self.depth >= self.max {
            return Err(Diagnostic {
                message: "formula depth limit exceeded".into(),
            });
        }
        match self.current.clone() {
            Tok::Number(n, u) => {
                self.next();
                Ok(Expr::Number(n, u))
            }
            Tok::String(s) => {
                self.next();
                Ok(Expr::String(s))
            }
            Tok::Op(Op::Sub) => {
                self.next();
                Ok(Expr::Unary(Box::new(self.primary()?)))
            }
            Tok::Ident(s) => {
                self.next();
                if self.current == Tok::L {
                    self.depth += 1;
                    self.next();
                    let mut a = Vec::new();
                    if self.current != Tok::R {
                        loop {
                            a.push(self.cmp()?);
                            if self.current != Tok::Comma {
                                break;
                            }
                            self.next();
                        }
                    }
                    if self.current != Tok::R {
                        return Err(Diagnostic {
                            message: "expected ')'".into(),
                        });
                    }
                    self.next();
                    self.depth -= 1;
                    Ok(Expr::Call(s, a))
                } else {
                    Ok(Expr::Reference(s))
                }
            }
            Tok::L => {
                self.depth += 1;
                self.next();
                let x = self.cmp()?;
                if self.current != Tok::R {
                    return Err(Diagnostic {
                        message: "expected ')'".into(),
                    });
                }
                self.next();
                self.depth -= 1;
                Ok(x)
            }
            _ => Err(Diagnostic {
                message: "expected expression".into(),
            }),
        }
    }
}
fn unit(s: &str) -> Option<(Unit, f64)> {
    match s.to_ascii_lowercase().as_str() {
        "" => Some((Unit::Number, 1.)),
        "in" => Some((Unit::Inches, 1.)),
        "cm" => Some((Unit::Inches, 1. / 2.54)),
        "mm" => Some((Unit::Inches, 1. / 25.4)),
        "pt" => Some((Unit::Inches, 1. / 72.)),
        "pica" => Some((Unit::Inches, 1. / 6.)),
        "ft" => Some((Unit::Inches, 12.)),
        "m" => Some((Unit::Inches, 100. / 2.54)),
        "deg" => Some((Unit::Radians, std::f64::consts::PI / 180.)),
        "rad" => Some((Unit::Radians, 1.)),
        "bool" => Some((Unit::Bool, 1.)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use vsdx_parse::{Shape, Sheet, parse_vsdx};

    fn limits() -> ParseLimits {
        ParseLimits::default()
    }
    fn number(formula: &str) -> Value {
        match evaluate(formula, &BTreeMap::new(), &limits()) {
            Evaluation::Evaluated(value) => value,
            value => panic!("expected value, got {value:?}"),
        }
    }

    #[test]
    fn parses_literals_references_and_precedence() {
        assert_eq!(number("1 + 2 * 3").number, 7.0);
        assert_eq!(number("2^3^2").number, 512.0);
        assert_eq!(number("-(1 + 2)").number, -3.0);
        assert!(matches!(
            parse("Geometry1.X2", &limits()),
            Ok(Expr::Reference(_))
        ));
        assert!(matches!(
            parse("Sheet.5!Width", &limits()),
            Ok(Expr::Reference(_))
        ));
        assert!(matches!(
            parse("IF(AND(1, 1), MIN(3, 2), 0)", &limits()),
            Ok(Expr::Call(_, _))
        ));
    }

    #[test]
    fn converts_units_and_rejects_mismatches() {
        let value = number("1 in + 25.4 mm");
        assert_eq!(value.unit, Unit::Inches);
        assert!((value.number - 2.0).abs() < 1e-12);
        assert!(matches!(
            evaluate("1 in + 1 deg", &BTreeMap::new(), &limits()),
            Evaluation::Error(_)
        ));
    }

    #[test]
    fn evaluates_display_functions_and_unsupported_calls() {
        assert_eq!(number("MOD(-3, 2)").number, 1.0);
        assert!((number("ATAN2(1, -1)").number - 3.0 * std::f64::consts::PI / 4.0).abs() < 1e-12);
        assert_eq!(number("ROUND(1.5)").number, 2.0);
        for formula in ["GUARD(1)", "SETATREF(1)", "NotAFunction(1)"] {
            assert!(matches!(
                evaluate(formula, &BTreeMap::new(), &limits()),
                Evaluation::Unsupported(_)
            ));
        }
    }

    #[test]
    fn detects_reference_cycles_and_depth() {
        let refs = BTreeMap::from([("A".into(), "B".into()), ("B".into(), "A".into())]);
        assert!(matches!(
            evaluate("A", &refs, &limits()),
            Evaluation::Error(_)
        ));
        let limits = ParseLimits {
            max_formula_depth: 2,
            ..limits()
        };
        assert!(parse("(((1)))", &limits).is_err());
    }

    #[test]
    fn corpus_formulas_parse_without_panics() {
        let Ok(directory) = std::env::var("VSDX_CORPUS_DIR") else {
            return;
        };
        let mut parsed = 0_usize;
        let mut unsupported = 0_usize;
        let mut failed = 0_usize;
        let mut examples = Vec::new();
        let mut unsupported_names = BTreeMap::new();
        for entry in fs::read_dir(directory).expect("read corpus directory") {
            let path = entry.expect("read corpus entry").path();
            if path
                .extension()
                .is_none_or(|extension| !extension.eq_ignore_ascii_case("vsdx"))
            {
                continue;
            }
            let package = parse_vsdx(&fs::read(&path).expect("read corpus file"))
                .expect("parse corpus package");
            for sheet in package
                .page_contents
                .values()
                .chain(package.master_contents.values())
            {
                for formula in formulas(sheet) {
                    match parse(formula, &limits()) {
                        Ok(expression) if has_unsupported(&expression) => {
                            unsupported += 1;
                            collect_unsupported(&expression, &mut unsupported_names);
                        }
                        Ok(_) => parsed += 1,
                        Err(_) => {
                            failed += 1;
                            if examples.len() < 12 {
                                examples.push(formula.to_owned());
                            }
                        }
                    }
                }
            }
        }
        eprintln!(
            "VSDX corpus formulas: parsed={parsed} unsupported={unsupported} failed={failed} total={}",
            parsed + unsupported + failed
        );
        eprintln!("VSDX corpus parse failures: {examples:?}");
        eprintln!("VSDX corpus unsupported functions: {unsupported_names:?}");
        assert_eq!(
            failed, 0,
            "every corpus formula must parse or classify unsupported"
        );
    }

    fn formulas(sheet: &Sheet) -> Vec<&str> {
        let mut values = sheet
            .cells()
            .filter_map(|cell| cell.formula.as_deref())
            .collect::<Vec<_>>();
        for section in sheet.sections() {
            for row in section.rows() {
                values.extend(row.cells().filter_map(|cell| cell.formula.as_deref()));
            }
        }
        for shape in sheet.shapes() {
            shape_formulas(shape, &mut values);
        }
        values
    }
    fn shape_formulas<'a>(shape: &'a Shape, values: &mut Vec<&'a str>) {
        values.extend(shape.cells().filter_map(|cell| cell.formula.as_deref()));
        for section in shape.sections() {
            for row in section.rows() {
                values.extend(row.cells().filter_map(|cell| cell.formula.as_deref()));
            }
        }
        for child in &shape.children {
            if let vsdx_parse::ShapeChild::Shapes(children) = child {
                for child in children {
                    if let vsdx_parse::ShapesChild::Shape(shape) = child {
                        shape_formulas(shape, values);
                    }
                }
            }
        }
    }
    fn has_unsupported(expression: &Expr) -> bool {
        match expression {
            Expr::Call(name, args) => {
                !matches!(
                    name.to_ascii_uppercase().as_str(),
                    "IF" | "AND"
                        | "OR"
                        | "NOT"
                        | "MIN"
                        | "MAX"
                        | "ABS"
                        | "INT"
                        | "ROUND"
                        | "CEILING"
                        | "FLOOR"
                        | "SQRT"
                        | "SIN"
                        | "COS"
                        | "TAN"
                        | "ATAN2"
                        | "PI"
                        | "MOD"
                        | "SUM"
                        | "TRUNC"
                        | "SIGN"
                ) || args.iter().any(has_unsupported)
            }
            Expr::Unary(value) => has_unsupported(value),
            Expr::Binary(left, _, right) => has_unsupported(left) || has_unsupported(right),
            _ => false,
        }
    }
    fn collect_unsupported(expression: &Expr, counts: &mut BTreeMap<String, usize>) {
        match expression {
            Expr::Call(name, args) => {
                if has_unsupported(&Expr::Call(name.clone(), Vec::new())) {
                    *counts.entry(name.to_ascii_uppercase()).or_default() += 1;
                }
                for argument in args {
                    collect_unsupported(argument, counts);
                }
            }
            Expr::Unary(value) => collect_unsupported(value, counts),
            Expr::Binary(left, _, right) => {
                collect_unsupported(left, counts);
                collect_unsupported(right, counts);
            }
            _ => {}
        }
    }
}
