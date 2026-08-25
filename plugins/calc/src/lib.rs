//! Inline calculator: {{2 km + 300 m}} renders as its result. The raw
//! expression reveals when the cursor touches it, like any inline.

wit_bindgen::generate!({ path: "../wit-v4", world: "extension" });

use supermd::extension::types as t;

/// Evaluate a Soulver-style expression: arithmetic with optional
/// units (length, mass, time) and `N% of X`.
pub fn evaluate(expr: &str) -> Result<String, String> {
    // `N% of X` sugar rewrites to (N/100) * X.
    if let Some((pct, rest)) = expr.split_once("% of ") {
        return evaluate(&format!("({pct}) / 100 * ({rest})"));
    }
    let tokens = tokenize(expr)?;
    let mut parser = Parser { tokens, pos: 0 };
    let value = parser.expr()?;
    if parser.pos != parser.tokens.len() {
        return Err("unexpected trailing input".to_string());
    }
    Ok(format_value(&value))
}

/// (dimension, factor to base unit). Base units: mm, g, s.
fn unit_info(name: &str) -> Option<(&'static str, f64)> {
    Some(match name {
        "mm" => ("length", 1.0),
        "cm" => ("length", 10.0),
        "m" => ("length", 1000.0),
        "km" => ("length", 1_000_000.0),
        "g" => ("mass", 1.0),
        "kg" => ("mass", 1000.0),
        "t" => ("mass", 1_000_000.0),
        "s" | "sec" => ("time", 1.0),
        "min" => ("time", 60.0),
        "h" | "hr" => ("time", 3600.0),
        _ => return None,
    })
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f64),
    Unit(&'static str, f64, String), // dimension, factor, display name
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Open,
    Close,
}

fn tokenize(src: &str) -> Result<Vec<Token>, String> {
    let mut out = Vec::new();
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' => i += 1,
            '+' => { out.push(Token::Plus); i += 1 }
            '-' => { out.push(Token::Minus); i += 1 }
            '*' | '×' => { out.push(Token::Star); i += 1 }
            '/' | '÷' => { out.push(Token::Slash); i += 1 }
            '^' => { out.push(Token::Caret); i += 1 }
            '(' => { out.push(Token::Open); i += 1 }
            ')' => { out.push(Token::Close); i += 1 }
            '0'..='9' | '.' => {
                let start = i;
                while i < chars.len()
                    && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == ',')
                {
                    i += 1;
                }
                let raw: String = chars[start..i].iter().filter(|&&c| c != ',').collect();
                let n: f64 = raw.parse().map_err(|_| format!("bad number {raw}"))?;
                out.push(Token::Number(n));
            }
            c if c.is_alphabetic() => {
                let start = i;
                while i < chars.len() && chars[i].is_alphanumeric() {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                let (dim, factor) =
                    unit_info(&word).ok_or_else(|| format!("unknown word {word}"))?;
                out.push(Token::Unit(dim, factor, word));
            }
            other => return Err(format!("unexpected character {other}")),
        }
    }
    if out.is_empty() {
        return Err("empty expression".to_string());
    }
    Ok(out)
}

/// A value with an optional dimension; `display` remembers the unit it
/// should print in (the first unit that appeared for that dimension).
#[derive(Clone, Debug)]
struct Value {
    base: f64, // in base units when dimensioned
    dimension: Option<&'static str>,
    display: Option<(String, f64)>, // (unit name, factor)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn expr(&mut self) -> Result<Value, String> {
        let mut left = self.term()?;
        while let Some(op) = self.peek().cloned() {
            match op {
                Token::Plus | Token::Minus => {
                    self.next();
                    let right = self.term()?;
                    if left.dimension != right.dimension {
                        return Err("cannot add values with different units".to_string());
                    }
                    left = Value {
                        base: if matches!(op, Token::Plus) {
                            left.base + right.base
                        } else {
                            left.base - right.base
                        },
                        dimension: left.dimension,
                        display: left.display.clone().or(right.display),
                    };
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn term(&mut self) -> Result<Value, String> {
        let mut left = self.power()?;
        while let Some(op) = self.peek().cloned() {
            match op {
                Token::Star | Token::Slash => {
                    self.next();
                    let right = self.power()?;
                    match (left.dimension, right.dimension) {
                        (_, None) => {
                            if matches!(op, Token::Slash) && right.base == 0.0 {
                                return Err("division by zero".to_string());
                            }
                            left.base = if matches!(op, Token::Star) {
                                left.base * right.base
                            } else {
                                left.base / right.base
                            };
                        }
                        (None, Some(_)) if matches!(op, Token::Star) => {
                            left = Value {
                                base: left.base * right.base,
                                dimension: right.dimension,
                                display: right.display,
                            };
                        }
                        _ => return Err("unsupported unit arithmetic".to_string()),
                    }
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn power(&mut self) -> Result<Value, String> {
        let base = self.unary()?;
        if matches!(self.peek(), Some(Token::Caret)) {
            self.next();
            let exp = self.unary()?;
            if base.dimension.is_some() || exp.dimension.is_some() {
                return Err("exponents need plain numbers".to_string());
            }
            return Ok(Value {
                base: base.base.powf(exp.base),
                dimension: None,
                display: None,
            });
        }
        Ok(base)
    }

    fn unary(&mut self) -> Result<Value, String> {
        if matches!(self.peek(), Some(Token::Minus)) {
            self.next();
            let mut v = self.unary()?;
            v.base = -v.base;
            return Ok(v);
        }
        self.atom()
    }

    fn atom(&mut self) -> Result<Value, String> {
        match self.next() {
            Some(Token::Number(n)) => {
                // A unit may follow a number: "2 km".
                if let Some(Token::Unit(dim, factor, name)) = self.peek().cloned() {
                    self.next();
                    return Ok(Value {
                        base: n * factor,
                        dimension: Some(dim),
                        display: Some((name, factor)),
                    });
                }
                Ok(Value { base: n, dimension: None, display: None })
            }
            Some(Token::Open) => {
                let v = self.expr()?;
                match self.next() {
                    Some(Token::Close) => Ok(v),
                    _ => Err("missing )".to_string()),
                }
            }
            other => Err(format!("expected a number, found {other:?}")),
        }
    }
}

fn format_value(v: &Value) -> String {
    match (&v.dimension, &v.display) {
        (Some(_), Some((name, factor))) => {
            format!("{} {}", format_number(v.base / factor), name)
        }
        _ => format_number(v.base),
    }
}

/// Trim float noise, group thousands.
fn format_number(n: f64) -> String {
    let rounded = (n * 1e9).round() / 1e9;
    let raw = if rounded == rounded.trunc() && rounded.abs() < 1e15 {
        format!("{}", rounded as i64)
    } else {
        let s = format!("{rounded:.6}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    };
    let (int_part, frac) = match raw.split_once('.') {
        Some((i, f)) => (i.to_string(), Some(f.to_string())),
        None => (raw, None),
    };
    let (sign, digits) = match int_part.strip_prefix('-') {
        Some(rest) => ("-", rest.to_string()),
        None => ("", int_part),
    };
    let grouped: String = digits
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|chunk| std::str::from_utf8(chunk).unwrap())
        .collect::<Vec<_>>()
        .join(",");
    match frac {
        Some(f) => format!("{sign}{grouped}.{f}"),
        None => format!("{sign}{grouped}"),
    }
}

struct Plugin;

impl Guest for Plugin {
    fn render_block(_: String, _: String, _: t::Theme) -> Result<String, String> {
        Err("unused".into())
    }
    fn run_command(_: String, _: t::CommandInput) -> Result<t::CommandOutput, String> {
        Err("unused".into())
    }
    fn render_inline(_id: String, matched: String) -> Result<String, String> {
        let inner = matched
            .trim()
            .trim_start_matches("{{")
            .trim_end_matches("}}")
            .trim();
        // Failures keep the raw text visible (inline errors stay raw).
        evaluate(inner)
    }
    fn format_document(d: String) -> Result<String, String> {
        Ok(d)
    }
    fn process_paste(_: String) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn export_document(_: String, _: String, _: t::Theme) -> Result<Vec<ExportFile>, String> {
        Err("unused".into())
    }
    fn render_view(_: String, _: String) -> Result<String, String> {
        Err("unused".into())
    }
    fn status_text(_: String) -> Result<String, String> {
        Err("unused".into())
    }
    fn render_template(_: String, _: TemplateContext) -> Result<TemplateFile, String> {
        Err("unused".into())
    }
    fn on_save(_: String, _: String) -> Result<Option<String>, String> {
        Ok(None)
    }
}

export!(Plugin);

#[cfg(test)]
mod tests {
    use super::evaluate;

    #[test]
    fn plain_arithmetic() {
        assert_eq!(evaluate("2 + 3 * 4").unwrap(), "14");
        assert_eq!(evaluate("(2 + 3) * 4").unwrap(), "20");
        assert_eq!(evaluate("10 / 4").unwrap(), "2.5");
        assert_eq!(evaluate("2^10").unwrap(), "1,024");
        assert_eq!(evaluate("-5 + 3").unwrap(), "-2");
    }

    #[test]
    fn thousands_are_grouped() {
        assert_eq!(evaluate("1200 * 1000").unwrap(), "1,200,000");
        assert_eq!(evaluate("1,500 + 500").unwrap(), "2,000");
    }

    #[test]
    fn length_units_convert_to_the_leading_unit() {
        assert_eq!(evaluate("2 km + 300 m").unwrap(), "2.3 km");
        assert_eq!(evaluate("300 m + 2 km").unwrap(), "2,300 m");
        assert_eq!(evaluate("1.5 m + 20 cm").unwrap(), "1.7 m");
        assert_eq!(evaluate("10 mm + 1 cm").unwrap(), "20 mm");
    }

    #[test]
    fn mass_and_time_units_work() {
        assert_eq!(evaluate("2 kg + 250 g").unwrap(), "2.25 kg");
        assert_eq!(evaluate("1 h + 30 min").unwrap(), "1.5 h");
        assert_eq!(evaluate("90 s + 1 min").unwrap(), "150 s");
    }

    #[test]
    fn unit_scaling_by_plain_numbers() {
        assert_eq!(evaluate("3 * 2.5 km").unwrap(), "7.5 km");
        assert_eq!(evaluate("10 km / 4").unwrap(), "2.5 km");
    }

    #[test]
    fn percent_of() {
        assert_eq!(evaluate("15% of 80").unwrap(), "12");
        assert_eq!(evaluate("20% of 5 km").unwrap(), "1 km");
    }

    #[test]
    fn errors_are_reported_not_guessed() {
        assert!(evaluate("2 km + 3 kg").is_err(), "mixed dimensions");
        assert!(evaluate("2 +").is_err());
        assert!(evaluate("hello").is_err());
        assert!(evaluate("1 / 0").is_err());
    }
}
