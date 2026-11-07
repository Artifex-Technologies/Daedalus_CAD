//! Dynamic Components, as SketchUp's: a component's attributes, each a value or a formula (`=LenX*2`), some of them shown to
//! the user as options. The built-in ones place and size the copy: `X`, `Y`, `Z` (where its origin is), `LenX`, `LenY`,
//! `LenZ` (its size along its own axes), `RotZ` (its turn about its blue axis, degrees). `onClick` (the Interact tool) holds
//! actions: `SET("name", value)`, `ALTER("name", a, b, ...)` (the next of those values), separated by `;`.
//!
//! How a copy is drawn (`model::dc_view`, used by drawing, export and picking): `Hidden` (true: not drawn), `Copies` (that
//! many more, at most `MAX_COPIES`, each placed by the same formulas with `COPY` = 1, 2, ...; the copy itself has COPY = 0)
//! and `Material` (a colour for its unpainted faces, as SketchUp's default material: `#rrggbb`, `r,g,b` or a name). A
//! formula may read the component a copy sits in: `Parent!LenX`, `Parent!Width`.
//!
//! Lengths are metres; a plain value may carry a unit (`300mm`, `2'6"`). Formulas: numbers, "text", `+ - * / ^`, `&` (joins
//! text), comparisons `= <> < > <= >=` (1 or 0), brackets, attribute names (any case), and IF, AND, OR, NOT, MIN, MAX, ABS,
//! SQRT, ROUND, FLOOR, CEILING, INT, SIN, COS, TAN, ASIN, ACOS, ATAN (degrees), PI.

use std::collections::HashMap;

/// One attribute: its name, its value or formula (a formula starts with `=`), and, for one the user may change, the label it
/// is shown with in Component Options.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub name: String,
    pub formula: String,
    pub label: Option<String>,
}

/// A worked-out value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f64),
    Text(String),
}

impl Value {
    pub fn num(&self) -> f64 {
        match self {
            Value::Num(n) => *n,
            Value::Text(t) => t.trim().parse().unwrap_or(0.0),
        }
    }
    pub fn text(&self) -> String {
        match self {
            Value::Num(n) => {
                let r = (n * 1e9).round() / 1e9;
                if r.fract() == 0.0 && r.abs() < 1e15 { format!("{}", r as i64) } else { format!("{r}") }
            }
            Value::Text(t) => t.clone(),
        }
    }
}

/// The names that place and size a copy.
pub const BUILT_INS: [&str; 7] = ["X", "Y", "Z", "LenX", "LenY", "LenZ", "RotZ"];

/// The names that say how a copy is drawn: Hidden (true: not drawn), Copies (that many more, placed with COPY = 1, 2, ...)
/// and Material (a colour for its unpainted faces).
pub const DRAWING: [&str; 3] = ["Hidden", "Copies", "Material"];

/// The most copies one component's Copies attribute makes.
pub const MAX_COPIES: usize = 500;

/// A Material value as a colour: `#rrggbb`, `#rgb`, `r,g,b` (0-255, also inside `rgb(...)`) or a common colour name.
pub fn parse_color(text: &str) -> Option<[u8; 3]> {
    let t = text.trim().to_ascii_lowercase();
    if let Some(h) = t.strip_prefix('#') {
        let v = |s: &str| u8::from_str_radix(s, 16).ok();
        return match h.len() {
            6 => Some([v(&h[0..2])?, v(&h[2..4])?, v(&h[4..6])?]),
            3 => Some([v(&h[0..1])? * 17, v(&h[1..2])? * 17, v(&h[2..3])? * 17]),
            _ => None,
        };
    }
    let inner = t.strip_prefix("rgb(").and_then(|x| x.strip_suffix(')')).unwrap_or(&t);
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if parts.len() == 3 {
        let mut rgb = [0u8; 3];
        for (k, p) in parts.iter().enumerate() {
            rgb[k] = p.parse::<f64>().ok().filter(|x| (0.0..=255.0).contains(x))?.round() as u8;
        }
        return Some(rgb);
    }
    const NAMES: [(&str, [u8; 3]); 24] = [
        ("black", [0, 0, 0]), ("white", [255, 255, 255]), ("red", [220, 40, 40]), ("green", [40, 160, 60]), ("blue", [40, 90, 220]),
        ("yellow", [240, 210, 40]), ("orange", [240, 140, 30]), ("purple", [130, 60, 180]), ("pink", [240, 150, 190]),
        ("brown", [130, 80, 40]), ("gray", [128, 128, 128]), ("grey", [128, 128, 128]), ("silver", [192, 192, 192]),
        ("gold", [212, 175, 55]), ("cyan", [40, 200, 220]), ("magenta", [210, 50, 180]), ("navy", [20, 30, 110]),
        ("teal", [20, 128, 128]), ("olive", [128, 128, 0]), ("maroon", [128, 20, 30]), ("beige", [230, 220, 190]),
        ("tan", [210, 180, 140]), ("wood", [160, 110, 60]), ("steel", [140, 150, 160]),
    ];
    NAMES.iter().find(|(n, _)| *n == t).map(|(_, c)| *c)
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Name(String),
    Op(String),
    Open,
    Close,
    Comma,
}

fn lex(s: &str) -> Result<Vec<Tok>, String> {
    let c: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch.is_ascii_digit() || (ch == '.' && c.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            while i < c.len() && (c[i].is_ascii_digit() || c[i] == '.') {
                i += 1;
            }
            if i < c.len() && (c[i] == 'e' || c[i] == 'E') && c.get(i + 1).is_some_and(|d| d.is_ascii_digit() || *d == '-' || *d == '+') {
                i += 2;
                while i < c.len() && c[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let t: String = c[start..i].iter().collect();
            out.push(Tok::Num(t.parse().map_err(|_| format!("{t} is not a number"))?));
        } else if ch == '"' {
            let mut t = String::new();
            i += 1;
            while i < c.len() && c[i] != '"' {
                t.push(c[i]);
                i += 1;
            }
            if i >= c.len() {
                return Err("a text has no closing \"".into());
            }
            i += 1;
            out.push(Tok::Str(t));
        } else if ch.is_alphabetic() || ch == '_' {
            let start = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                i += 1;
                // Parent!LenX: an attribute of the component this copy sits in
                if i + 1 < c.len() && c[i] == '!' && c[i + 1].is_alphabetic() {
                    i += 1;
                }
            }
            out.push(Tok::Name(c[start..i].iter().collect()));
        } else if ch == '(' {
            out.push(Tok::Open);
            i += 1;
        } else if ch == ')' {
            out.push(Tok::Close);
            i += 1;
        } else if ch == ',' {
            out.push(Tok::Comma);
            i += 1;
        } else {
            let two: String = c[i..(i + 2).min(c.len())].iter().collect();
            if ["<=", ">=", "<>"].contains(&two.as_str()) {
                out.push(Tok::Op(two));
                i += 2;
            } else if "+-*/^&=<>".contains(ch) {
                out.push(Tok::Op(ch.to_string()));
                i += 1;
            } else {
                return Err(format!("{ch} does not belong in a formula"));
            }
        }
    }
    Ok(out)
}

/// The values formulas look up: by name in lower case.
pub type Lookup<'a> = dyn FnMut(&str) -> Result<Value, String> + 'a;

struct Parser<'a, 'b> {
    toks: Vec<Tok>,
    at: usize,
    look: &'a mut Lookup<'b>,
}

impl Parser<'_, '_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.at)
    }
    fn op(&self, o: &str) -> bool {
        matches!(self.peek(), Some(Tok::Op(x)) if x == o)
    }
    fn compare(&mut self) -> Result<Value, String> {
        let a = self.additive()?;
        for o in ["<=", ">=", "<>", "=", "<", ">"] {
            if self.op(o) {
                self.at += 1;
                let b = self.additive()?;
                let r = match (&a, &b) {
                    (Value::Text(x), Value::Text(y)) => match o {
                        "=" => x.eq_ignore_ascii_case(y),
                        "<>" => !x.eq_ignore_ascii_case(y),
                        _ => false,
                    },
                    _ => {
                        let (x, y) = (a.num(), b.num());
                        let eq = (x - y).abs() <= 1e-9 * (1.0 + x.abs().max(y.abs()));
                        match o {
                            "=" => eq,
                            "<>" => !eq,
                            "<" => x < y && !eq,
                            ">" => x > y && !eq,
                            "<=" => x < y || eq,
                            _ => x > y || eq,
                        }
                    }
                };
                return Ok(Value::Num(if r { 1.0 } else { 0.0 }));
            }
        }
        Ok(a)
    }
    fn additive(&mut self) -> Result<Value, String> {
        let mut a = self.term()?;
        loop {
            if self.op("+") || self.op("-") {
                let plus = self.op("+");
                self.at += 1;
                let b = self.term()?;
                a = Value::Num(if plus { a.num() + b.num() } else { a.num() - b.num() });
            } else if self.op("&") {
                self.at += 1;
                let b = self.term()?;
                a = Value::Text(a.text() + &b.text());
            } else {
                return Ok(a);
            }
        }
    }
    fn term(&mut self) -> Result<Value, String> {
        let mut a = self.power()?;
        while self.op("*") || self.op("/") {
            let times = self.op("*");
            self.at += 1;
            let b = self.power()?.num();
            a = Value::Num(if times { a.num() * b } else if b == 0.0 { return Err("a division by zero".into()) } else { a.num() / b });
        }
        Ok(a)
    }
    fn power(&mut self) -> Result<Value, String> {
        let a = self.unary()?;
        if self.op("^") {
            self.at += 1;
            let b = self.power()?.num();
            return Ok(Value::Num(a.num().powf(b)));
        }
        Ok(a)
    }
    fn unary(&mut self) -> Result<Value, String> {
        if self.op("-") {
            self.at += 1;
            return Ok(Value::Num(-self.unary()?.num()));
        }
        if self.op("+") {
            self.at += 1;
            return self.unary();
        }
        self.primary()
    }
    fn args(&mut self) -> Result<Vec<Value>, String> {
        let mut out = Vec::new();
        if matches!(self.peek(), Some(Tok::Close)) {
            self.at += 1;
            return Ok(out);
        }
        loop {
            out.push(self.compare()?);
            match self.peek() {
                Some(Tok::Comma) => self.at += 1,
                Some(Tok::Close) => {
                    self.at += 1;
                    return Ok(out);
                }
                _ => return Err("a function's values are not closed with )".into()),
            }
        }
    }
    fn primary(&mut self) -> Result<Value, String> {
        let t = self.peek().cloned().ok_or("the formula ends too soon")?;
        self.at += 1;
        match t {
            Tok::Num(n) => Ok(Value::Num(n)),
            Tok::Str(s) => Ok(Value::Text(s)),
            Tok::Open => {
                let v = self.compare()?;
                if self.peek() != Some(&Tok::Close) {
                    return Err("a ( is not closed".into());
                }
                self.at += 1;
                Ok(v)
            }
            Tok::Name(n) => {
                if self.peek() == Some(&Tok::Open) {
                    self.at += 1;
                    let a = self.args()?;
                    return call(&n, &a);
                }
                if n.eq_ignore_ascii_case("pi") {
                    return Ok(Value::Num(std::f64::consts::PI));
                }
                (self.look)(&n.to_lowercase())
            }
            other => Err(format!("{other:?} cannot start a value")),
        }
    }
}

fn call(name: &str, a: &[Value]) -> Result<Value, String> {
    let n = |i: usize| a.get(i).map(Value::num).unwrap_or(0.0);
    let deg = |x: f64| x.to_radians();
    let f = name.to_ascii_uppercase();
    let want = |k: usize| if a.len() < k { Err(format!("{f} needs {k} values")) } else { Ok(()) };
    Ok(Value::Num(match f.as_str() {
        "IF" => {
            want(2)?;
            return Ok(if n(0) != 0.0 { a[1].clone() } else { a.get(2).cloned().unwrap_or(Value::Num(0.0)) });
        }
        "AND" => (a.iter().all(|v| v.num() != 0.0)) as i32 as f64,
        "OR" => (a.iter().any(|v| v.num() != 0.0)) as i32 as f64,
        "NOT" => (n(0) == 0.0) as i32 as f64,
        "MIN" => a.iter().map(Value::num).fold(f64::INFINITY, f64::min),
        "MAX" => a.iter().map(Value::num).fold(f64::NEG_INFINITY, f64::max),
        "ABS" => n(0).abs(),
        "SQRT" => n(0).sqrt(),
        "ROUND" => {
            let p = 10f64.powi(n(1) as i32);
            (n(0) * p).round() / p
        }
        "FLOOR" | "INT" => n(0).floor(),
        "CEILING" => n(0).ceil(),
        "SIN" => deg(n(0)).sin(),
        "COS" => deg(n(0)).cos(),
        "TAN" => deg(n(0)).tan(),
        "ASIN" => n(0).asin().to_degrees(),
        "ACOS" => n(0).acos().to_degrees(),
        "ATAN" => n(0).atan().to_degrees(),
        _ => return Err(format!("{name} is not a function Daedalus knows")),
    }))
}

/// Works out `formula` (the part after `=`), looking names up with `look`.
pub fn eval(formula: &str, look: &mut Lookup) -> Result<Value, String> {
    let toks = lex(formula)?;
    let mut p = Parser { toks, at: 0, look };
    let v = p.compare()?;
    if p.at != p.toks.len() {
        return Err(format!("the formula has more after its end: {:?}", p.toks[p.at]));
    }
    Ok(v)
}

/// A plain value: a length (any unit, metres back), a number, or text.
fn literal(text: &str) -> Value {
    let t = text.trim();
    if let Ok(n) = t.parse::<f64>() {
        return Value::Num(n);
    }
    match crate::units::parse(t, crate::units::Units::Metres) {
        Some(m) => Value::Num(m),
        None => Value::Text(t.to_string()),
    }
}

/// Every attribute's value (lower-case names), formulas worked out in the right order; `given` supplies the built-ins
/// an attribute does not set (where the copy is now). A formula that goes wrong (or refers round in a circle) is an error
/// naming the attribute.
pub fn evaluate(attrs: &[Attribute], given: &HashMap<String, f64>) -> Result<HashMap<String, Value>, String> {
    fn get(name: &str, attrs: &[Attribute], given: &HashMap<String, f64>, done: &mut HashMap<String, Value>, busy: &mut Vec<String>) -> Result<Value, String> {
        if let Some(v) = done.get(name) {
            return Ok(v.clone());
        }
        let Some(a) = attrs.iter().find(|a| a.name.to_lowercase() == name) else {
            return given.get(name).map(|&n| Value::Num(n)).ok_or(format!("there is no attribute called {name}"));
        };
        if busy.iter().any(|b| b == name) {
            return Err(format!("{} refers back to itself", a.name));
        }
        busy.push(name.to_string());
        let v = match a.formula.trim().strip_prefix('=') {
            Some(f) => {
                let mut look = |n: &str| get(n, attrs, given, done, busy);
                eval(f, &mut look).map_err(|e| format!("{}: {e}", a.name))?
            }
            None => literal(&a.formula),
        };
        busy.pop();
        done.insert(name.to_string(), v.clone());
        Ok(v)
    }
    let mut done = HashMap::new();
    for a in attrs {
        if !a.name.eq_ignore_ascii_case("onclick") {
            get(&a.name.to_lowercase(), attrs, given, &mut done, &mut Vec::new())?;
        }
    }
    Ok(done)
}

/// Interact: runs the `onClick` actions, changing the attributes they name to plain values. Returns whether any changed.
pub fn on_click(attrs: &mut Vec<Attribute>, given: &HashMap<String, f64>) -> Result<bool, String> {
    let Some(script) = attrs.iter().find(|a| a.name.eq_ignore_ascii_case("onclick")).map(|a| a.formula.trim().trim_start_matches('=').to_string()) else {
        return Ok(false);
    };
    let mut changed = false;
    for action in split_top(&script, ';') {
        let action = action.trim();
        if action.is_empty() {
            continue;
        }
        let open = action.find('(').ok_or(format!("{action} is not an action"))?;
        let verb = action[..open].trim().to_ascii_uppercase();
        let inner = action[open + 1..].trim_end().strip_suffix(')').ok_or(format!("{action} is not closed with )"))?;
        let parts = split_top(inner, ',');
        let target = parts.first().map(|t| t.trim().trim_matches('"').to_string()).filter(|t| !t.is_empty()).ok_or("an action names no attribute")?;
        let values = evaluate(attrs, given)?;
        let mut look = |n: &str| values.get(n).cloned().or_else(|| given.get(n).map(|&x| Value::Num(x))).ok_or(format!("there is no attribute called {n}"));
        let new = match verb.as_str() {
            "SET" => eval(parts.get(1).ok_or("SET needs a value")?, &mut look)?,
            "ALTER" | "ANIMATE" | "ANIMATESLOW" | "ANIMATEFAST" => {
                let choices: Vec<Value> = parts[1..].iter().map(|p| eval(p, &mut look)).collect::<Result<_, _>>()?;
                if choices.is_empty() {
                    return Err("ALTER needs values to go through".into());
                }
                let now = look(&target.to_lowercase()).unwrap_or(Value::Num(f64::NAN));
                let at = choices.iter().position(|c| match (c, &now) {
                    (Value::Num(a), Value::Num(b)) => (a - b).abs() <= 1e-9 * (1.0 + a.abs()),
                    (a, b) => a.text() == b.text(),
                });
                choices[at.map_or(0, |k| (k + 1) % choices.len())].clone()
            }
            other => return Err(format!("{other} is not an action Daedalus knows (SET, ALTER)")),
        };
        let text = new.text();
        match attrs.iter_mut().find(|a| a.name.eq_ignore_ascii_case(&target)) {
            Some(a) => a.formula = text,
            None => attrs.push(Attribute { name: target, formula: text, label: None }),
        }
        changed = true;
    }
    Ok(changed)
}

/// `s` split at `sep` where it is not inside brackets or quotes.
fn split_top(s: &str, sep: char) -> Vec<String> {
    let (mut out, mut now, mut depth, mut quoted) = (Vec::new(), String::new(), 0, false);
    for ch in s.chars() {
        match ch {
            '"' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => depth -= 1,
            c if c == sep && depth == 0 && !quoted => {
                out.push(std::mem::take(&mut now));
                continue;
            }
            _ => {}
        }
        now.push(ch);
    }
    out.push(now);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attr(n: &str, f: &str) -> Attribute {
        Attribute { name: n.into(), formula: f.into(), label: None }
    }

    #[test]
    fn formulas_work_out_in_order() {
        let attrs = vec![attr("LenX", "=Width*2"), attr("Width", "300mm"), attr("Big", "=IF(LenX>0.5, \"yes\", \"no\")"), attr("Shelves", "=ROUND(LenZ/0.4, 0)")];
        let given: HashMap<String, f64> = [("lenz".to_string(), 1.8)].into();
        let v = evaluate(&attrs, &given).unwrap();
        assert!((v["lenx"].num() - 0.6).abs() < 1e-12);
        assert_eq!(v["big"], Value::Text("yes".into()));
        assert_eq!(v["shelves"].num(), 5.0, "1.8 / 0.4 = 4.5, rounded");
        assert!((eval("2^3 + -1 * (4 - 1)", &mut |_| Err("none".into())).unwrap().num() - 5.0).abs() < 1e-12);
        assert!((eval("COS(60)", &mut |_| Err("none".into())).unwrap().num() - 0.5).abs() < 1e-12);
    }

    #[test]
    fn a_circle_of_formulas_is_an_error() {
        let attrs = vec![attr("A", "=B+1"), attr("B", "=A+1")];
        assert!(evaluate(&attrs, &HashMap::new()).unwrap_err().contains("itself"));
        assert!(evaluate(&[attr("A", "=Nope")], &HashMap::new()).is_err());
    }

    #[test]
    fn on_click_alters_and_sets() {
        let mut attrs = vec![attr("Open", "0"), attr("onClick", "ALTER(\"Open\", 0, 1); SET(\"LenX\", Open*2)")];
        assert!(on_click(&mut attrs, &HashMap::new()).unwrap());
        assert_eq!(attrs[0].formula, "1");
        assert_eq!(attrs.iter().find(|a| a.name == "LenX").unwrap().formula, "2", "SET sees the value ALTER just gave");
        on_click(&mut attrs, &HashMap::new()).unwrap();
        assert_eq!(attrs[0].formula, "0", "and back round");
    }
}
